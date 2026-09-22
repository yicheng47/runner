# 701 — Desktop notifications when an agent needs you

> Tracking issue: [#701](https://github.com/yicheng47/runner/issues/701)
> Priority: P2, milestone 0.12. Platforms: macOS and Windows.
> Decision, 2026-09-22: follow Zed's agent notification, a popup window Runner draws itself, not the OS notification center.

## Motivation

Runner shows that an agent needs you only inside its own windows: the sidebar's unread and attention dots, and the status control on each pane. Once Runner is behind an editor or a browser, an agent waiting on an approval, a question, or a finished turn waits until you happen to look back. With a crew and several chats running at once, that idle time is the main cost of running agents in parallel.

[#130](https://github.com/yicheng47/runner/issues/130) asked for macOS notifications in the Tauri era and was cut down to the in-app status signal before any notification shipped. Notifications were never built.

## Model: Zed's agent notification

Zed has the same problem for its agent panel and solves it without the OS notification center. Read against the local clone (`~/repos/gui/zed` at `db7c1d38c8`, `crates/agent_ui/src/ui/agent_notification.rs` and `conversation_view.rs`):

- **Window**: a 450 × 72 `WindowKind::PopUp` window, placed 16 px from the right edge and 48 px below the top of the display. It opens with `focus: false`, `is_movable: false`, no titlebar, a transparent background and client decorations.
- **Content**: an icon, the thread title, a caption with the reason, and the project name, beside a **View** button (accent) and a **Dismiss** button.
- **Triggers**: "Waiting for tool confirmation", "Waiting for input", "New message" or "Finished running tools", and "Agent stopped due to an error".
- **Only when unseen**: nothing shows when the window is active and the panel is showing that thread. Each thread shows one popup at most.
- **Setting**: `notify_when_agent_waiting` is `primary_screen` (the default), `all_screens` (one popup per display) or `never`. Each popup also calls `window.request_attention()`.
- **Dismissal**: View activates Zed, brings the window forward, reveals the panel and loads the thread. Dismiss closes the popup. The popup also closes itself when window activation makes the thread visible. It has no timeout.
- **Sound**: `play_sound_when_agent_done` is a separate setting and defaults to `never`.

Runner follows this model because it is built on the same framework, and a popup avoids everything OS notifications would bring:

- **App identity**: OS notifications need a registered app identity on each platform: an app bundle on macOS, and an AppUserModelID shortcut on Windows. `make run` builds have neither.
- **Permission and routing**: macOS asks the user for permission, and a click has to find its way back through the OS to the right session.
- **Consistency**: the popup looks and behaves the same on both platforms.

The cost: the popup has no Notification Center history, and it ignores Focus and Do Not Disturb. The Off setting covers that for now.

## Behavior

### Triggers

A popup fires when a session *enters* one of these states. It does not fire again on every status refresh, and a session that is already waiting when Runner starts does not fire; the first load sets the baseline.

| Trigger | Signal | Direct chat | Mission slot | Caption |
| --- | --- | --- | --- | --- |
| Approval wait | a new `WaitReason::Approval` in `observation.interactions` | yes | yes | Waiting for approval |
| Input wait | a new `WaitReason::Answer` or `WaitReason::Unknown` | yes | yes | Waiting for input |
| Turn finished unseen | `status.unread_since` becomes set, as recorded by `record_session_completion` | yes | no | Finished |
| Failure | a failed turn outcome (`failed_since`), or `Lifecycle::Error` (`error_since`) | yes | yes | Response failed / Stopped with an error |
| Mission question | a `human_question` event is appended, keyed to its `ask_human` | — | yes | @asker asked you a question |

Shell sessions never notify, which matches `record_session_completion` skipping them. Mission slots do not notify when a turn finishes, because a working crew finishes turns constantly; the feed and the lead's `ask_human` are how a mission reaches you.

### When a popup shows

A popup fires only when no focused Runner window displays the session. That is `WindowRegistry::any_focused_displaying` (`crates/runner-backend/src/windows.rs`), the same check that decides whether a finished turn counts as unread. The rule covers three cases:

- **Runner in the background**: the popup shows.
- **Runner in front, session in another tab or window**: the popup shows.
- **Session in a visible pane of the focused window**: nothing shows.

For a mission slot, "displayed" means the mission workspace is showing that slot. Confirm that the registry's subjects cover this before relying on it.

### The popup

- **Size and position**: Zed's size and position, 450 × 72 at the top right of the display: 16 px from the right edge and 48 px below the top, which clears the macOS menu bar. Windows uses the same corner.
- **Content**: the runtime icon (`ChatIcon::for_runtime`) and the session's title as the sidebar shows it. Below the title are the caption and the project name, or the mission title for a slot, separated by a dot. **View** is the primary button and **Dismiss** the secondary. The popup follows Runner's theme.
- **Window behavior**: the popup never takes focus. It stays out of the Dock, the taskbar, ⌘Tab / Alt-Tab and Mission Control, and it floats above normal windows.
- **Stacking**: each session has at most one popup, and a newer trigger for the same session updates its caption in place. Popups from several sessions stack down from the top-right corner with an 8 px gap, newest on top. At most three are visible; a fourth replaces the oldest, whose session keeps its sidebar attention dot.
- **All screens**: each display gets its own popup for the session, and they all close together.

### Dismissal

- **View**:
  - Brings Runner forward, raises the window that holds the session, activates the tab and focuses the pane, using the paths a sidebar pick already uses: `open_chat_session` (`surfaces/sidebar/activation.rs:176`), `PaneLayout::activate_session`, and `focus_other_window` (`main.rs:1586`).
  - A mission slot opens the mission workspace on that slot.
  - A session that no window holds opens in the main window, as a sidebar pick would.
  - Then all of the session's popups close.
- **Dismiss**: closes the session's popups. The sidebar dots stay.
- **Automatically**, when:
  - the session comes into view: a window gains focus, or the tab or pane switches to it;
  - the wait clears: the approval is given, or the user types;
  - the session is archived or closed;
  - the setting changes to Off.
- **No timeout**: like Zed's, the popup waits for you.

### Dock bounce and taskbar flash

gpui-ce 0.3.3 has no `request_attention`, so this goes in a `platform_ui` function next to the window chrome. On macOS it calls `NSApplication requestUserAttention:` with `NSInformationalRequest`, which bounces the Dock icon once. On Windows it calls `FlashWindowEx` with `FLASHW_TRAY | FLASHW_TIMERNOFG` on the Runner window, which flashes until the window comes forward. It fires with a popup only when Runner is not the active app, and the Off setting disables it too.

### Setting

Settings → General (`surfaces/settings_page.rs:1965`) gains a row, **Notify when an agent needs you**, with three options: Primary screen (the default), All screens, and Off. It is stored with the other app settings in `app_settings.rs`, as `primary_screen`, `all_screens` or `off`.

## Implementation notes

- **Where the code goes**: a notification controller owned by `NativeRoot` in `crates/runner-app/src/notifications.rs`, and the popup view in `crates/runner-app/src/ui/desktop_notification.rs`.
- **Trigger detection**: a pure function from the previous and next `AppStore::session_statuses` (`app_store.rs:254`), plus the newly appended mission events, to a list of `(session_id, reason)`. It runs on the refreshes that `StoreRefreshKind::for_event` already routes (`session/status`, and `event/appended` for `human_question`), so the backend and the socket do not change. Keeping it pure keeps the edge rules unit-testable.
- **Window options**: copy Zed's: `focus: false`, `show: true`, `kind: WindowKind::PopUp`, `is_movable: false`, `display_id`, a transparent background, client decorations and no titlebar.
- **Popups are not Runner windows**: they must never register in the `WindowRegistry`, claim a subject or count as focused. The window-activation path has to ignore them, so that clicking View does not remap which window owns a session before the routing runs.
- **Platform checks**: in gpui-ce, `WindowKind::PopUp` sets extended window styles on Windows (`platform/windows/window.rs:407`) and `NSPopUpWindowLevel` on macOS (`platform/mac/window.rs:805`). Two things are unverified:
  - that the Windows popup stays out of the taskbar and Alt-Tab;
  - that the macOS popup appears over another app's fullscreen Space.

  If either fails, fix it in `platform_ui`; on macOS that means the `canJoinAllSpaces` and `fullScreenAuxiliary` collection behaviours.

## Non-goals

- The macOS Notification Center or Windows notifications, and permission prompts.
- Sound.
- Settings for each trigger or each session, snoozing, and Focus / Do Not Disturb integration.
- Notification history, a tray or menu-bar icon, and a Dock badge count. The place you go to check what needs you is the Activity / Needs you view, [#552](https://github.com/yicheng47/runner/issues/552); this feature is the alert that comes to you.
- A popup for each crew turn that finishes.

## Implementation phases

1. **Design**: `design/specs/701-desktop-notifications.pen` with the popup in light and dark, a stack of three, and the Settings row. It is reviewed before any code.
2. **Direct chats**: the controller and trigger detection, the popup window, View routing across windows, automatic dismissal, and the setting.
3. **Missions and attention**: mission triggers (`human_question`, waits in a slot, crashes), View into the mission workspace, and the Dock bounce and taskbar flash.

## Verification

- **Unit tests** for trigger detection:
  - it fires only when a state is entered, and the first load sets the baseline;
  - shells never fire, and mission slots never fire for a finished turn;
  - the visibility rule, the three-popup cap, one popup per session, and Off.
- **Checks**: `runner-app` tests and workspace clippy.
- **Manual pass on macOS and Windows**:
  - With Runner behind another app, a direct chat that reaches an approval prompt shows one popup at the top right of the primary display, without taking focus from the app you are typing in. The Dock bounces once, or the taskbar button flashes.
  - View brings Runner forward on the right window, tab and pane, including for a session in a second window (⇧⌘N).
  - With Runner in front and the session in another tab, a popup shows, and switching to that tab closes it. Approving in the terminal closes it too.
  - Three chats finishing at once give three stacked popups; a fourth replaces the oldest.
  - With All screens set and two displays, each display shows a popup, and View on either one closes both.
  - In a mission, the lead's `ask_human` shows "@lead asked you a question" and View opens the mission workspace. Crew turns finishing show nothing.
  - The popup is absent from the Dock, the taskbar, ⌘Tab / Alt-Tab and Mission Control. On macOS it shows over a fullscreen app, or the gap is written down here.
  - With Off, nothing shows and nothing bounces.
