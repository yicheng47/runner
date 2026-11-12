# 701 — Desktop notifications when an agent needs you

Historical design recovery (2026-11-12): superseded desktop spec canvases referenced in this record are preserved in `YeelinksCo/yeelinks_design` at revision `4605d1e43ab6835a555fdd3957469a6b55ddaec8`, under `runner/desktop/specs/` (including `archive/`). Their original frame ids resolve there; current product designs are in `runner/desktop/runner.pen`.

> Tracking issue: [#701](https://github.com/yicheng47/runner/issues/701)
> Priority: P2, milestone 0.16 (moved from 0.13 to 0.14 on 2026-10-07, renumbered 0.16 on 2026-10-10). Platforms: macOS and Windows.
> Decision, 2026-10-13: use the operating system's own notifications, Notification Center on macOS and toast notifications on Windows, posted through GPUI's system notification API. This replaces the 2026-09-22 decision to copy Zed's self-drawn popup window.
> Defaults, Jason, 2026-10-13 (latest correction): post only while Runner is in the background; chat input, completion and errors on; the four independent mission switches off by default. Mission turn completion is replaced by a whole-mission Busy → Idle edge.
> Design: `runner/desktop/specs/701-desktop-notifications.pen` in the private `yeelinks_design` repo, revised and reviewed with Jason in Pencil before implementation. The panes are `K8FVAY` (dark) and `gYgga` (light), with all seven event rows always visible; the final design is committed on main as `ce2b8f81`, and the System row's permission states (`Feeq7` dark, `w4PN2` light) as `93b125ea`. The system draws the notification, so the design covers the Settings → Notifications pane (`K8FVAY`, with the new nav row in `cmp/SettingsNav`) and how each trigger fills the notification's title and body, drawn as macOS banners (`w9LiW` dark, `Ozj1d` light).

## Motivation

Runner shows that an agent needs you only inside its own windows: the sidebar's unread and attention dots, and the status control on each pane. Once Runner is behind an editor or a browser, an agent waiting on an approval, a question, or a finished turn waits until you happen to look back. With a crew and several chats running at once, that idle time is the main cost of running agents in parallel.

[#130](https://github.com/yicheng47/runner/issues/130) asked for macOS notifications in the Tauri era and was cut down to the in-app status signal before any notification shipped. Notifications were never built.

## Why native notifications

On macOS, apps normally alert through Notification Center; Slack, Mail, Messages and iTerm2 all do. A popup that an app draws itself outside its own windows is unusual. Zed's agent notification is the main example, and the first version of this spec copied it. Jason chose the native route on 2026-10-13 after reviewing the popup design.

Native notifications bring for free what a self-drawn popup would have to imitate or would lack:

- **Platform look**: the Mac banner on macOS and a toast on Windows, including the app's icon and name.
- **User control**: Focus and Do Not Disturb, Notification Center history, and the per-app banner style in System Settings.

They do not need Swift, an extension or new dependencies. `gpui-pre` 0.3.7, the GPUI Runner pins, already provides the API:

- **Calls**: `App::show_system_notification`, `App::dismiss_system_notification`, `App::on_system_notification_response` and `App::set_app_identity` (`gpui-pre/src/app.rs:1638`, the platform trait at `platform.rs:323`).
- **Notification**: a `SystemNotification` has a `tag`, a `title`, a `body` and optional action buttons. Posting with an existing tag replaces that notification, and a click returns its tag.
- **macOS**: it uses `UNUserNotificationCenter` (`gpui-pre-macos/src/system_notifications.rs`). It requests authorization on the first post, shows banners even while Runner is frontmost, and does nothing when the process is not in an app bundle instead of aborting.
- **Windows**: it uses WinRT toasts (`gpui-pre-windows/src/system_notifications.rs`). With `set_app_identity` it sets the process AppUserModelID and registers it under `HKCU\Software\Classes\AppUserModelId`, so an unpackaged build can show toasts too.

The costs, accepted:

- **macOS needs a bundle**: a plain `make run` build is not in an app bundle and posts nothing. QA and testing use a wrapped `.app`; see Verification.
- **Permission prompt**: Runner requests permission at launch, so notifications start working only once the user allows them. macOS lists an app under System Settings → Notifications only after it has requested authorization; waiting for GPUI's request on the first post left Runner missing from that list, and Open System Settings landing on a page without it, until something had notified (Jason, 2026-10-14, found before 0.16.1 shipped). The system prompts only while the answer is undecided; every later launch's request returns the saved answer without a prompt. On current macOS the request is itself a notification whose buttons show only on hover, so it is easy to miss, but Runner is listed either way. The System row always opens System Settings and has no Allow button of its own (Jason, 2026-10-14). After Don't Allow, nothing shows until the user allows Runner in System Settings. Windows shows toasts without asking.
- **Less control over presentation**: the system decides placement and stacking, and the notification has two lines of text and no custom styling.

## Behavior

### Triggers

A notification posts when a session *enters* one of these states. It does not post again on every status refresh, and a session that is already waiting when Runner starts does not post; the first load sets the baseline.

| Trigger | Signal | Status text |
| --- | --- | --- |
| Approval wait | a new `WaitReason::Approval` in `observation.interactions` | Approval needed |
| Answer wait | a new `WaitReason::Answer` | Answer needed |
| Input wait | a new `WaitReason::Unknown` | Needs you |
| Chat turn finished | `status.unread_since` becomes set, as recorded by `record_session_completion` | Finished |
| Mission goes idle | `MissionSummary.activity` changes from `Busy` to `Idle`; every live slot’s latest status is idle | Every crew member is idle |
| Failed turn | a failed turn outcome (`failed_since`) | Response failed |
| Process exited | `Lifecycle::Error` (`error_since`) | Error · exit code N |
| Mission question | a `human_question` event is appended, keyed to its `ask_human` | @asker asked you |

The waits and failures use the short labels of the pane status control (`StatusKind::label` in `ui/agent_status.rs`), so the notification reads the same as the pane you land on. The exit code is the one detail added to the plain Error label.

Each trigger has its own switch in Settings → Notifications, separately for chats and missions; see the table there. Shell sessions never notify, which matches `record_session_completion` skipping them.

### When a notification posts

A notification posts only while Runner is in the background and its event switch is on. Any focused Runner window suppresses new notifications, including when the relevant chat, mission, slot or question is in another tab or window. Edges observed while Runner is frontmost are consumed; moving Runner into the background does not replay them. There is no additional mode switch.

Chats notify for input, turn completion and errors by default. Each mission event has its own switch, all off by default and always visible. Mission slots never notify a finished turn. The whole mission can notify one Busy → Idle transition when Goes idle is on.

Withdrawal still follows what comes into view: every pane of the focused chat tab, the open mission slot tab, the mission feed for a question, and any tab of the mission workspace for its idle alert. The controller reads the windows directly so the unfocused pane of a visible split also counts. Returning to Runner on a different tab suppresses new alerts; selecting the alerted target withdraws its existing notification.

### Content

The system shows Runner's icon and name above two lines of text that Runner supplies:

| Trigger | Title | Body |
| --- | --- | --- |
| Direct chat | the chat title as the sidebar shows it | status text · project, e.g. "Approval needed · runner" |
| Mission slot | `@handle` · mission title | status text, e.g. "Approval needed" |
| Mission question | "@asker asked you" · mission title | the question itself; the system wraps and cuts it |
| Mission goes idle | mission title | Every crew member is idle |

Notifications carry no action buttons. Clicking the notification is View, and the system's own close control is Dismiss. They carry no sound.

### Tags and replacement

The tag is the session id, `question:<event id>` for a mission question, or `mission:<mission id>:idle` for mission idle, so each session has at most one notification. A newer trigger for the same session replaces the old one in place.

### Clicking a notification

The response callback, registered once at startup, receives the tag:

- **Direct chat**: Runner comes forward, raises the window that holds the session, activates the tab and focuses the pane. It uses the paths a sidebar pick already uses: `open_chat_session` (`surfaces/sidebar/activation.rs:176`), `PaneLayout::activate_session` (`pane_layout.rs:714`) and `focus_other_window` (`main.rs:1884`). A session that no window holds opens in the main window, as a sidebar pick would.
- **Mission slot**: the mission workspace opens on that slot.
- **Mission question**: the mission workspace opens on the feed with the ask card.
- **Mission goes idle**: the mission workspace opens on the feed, including when it is already loaded on a slot tab.

A click on a notification left over from an earlier run brings Runner forward; routing it to its session is best-effort.

### Withdrawing a notification

Runner calls `dismiss_system_notification` for a session when:

- the session comes into view: a window gains focus, or the tab or pane switches to it;
- the wait clears: the approval is given, or the user types;
- the session is archived or closed;
- the switch for its event is turned off;
- a mission idle alert clears when the mission becomes busy, has no live activity, is archived or disappears;
- Runner quits, because nothing is left to route a click or to clear the notification later.

On macOS this removes the notification from the screen and from Notification Center. On Windows it hides the toasts posted by the running process, and older ones age out of the action centre.

### Settings → Notifications

Notifications get their own Settings pane rather than a row in General, and users choose which events notify them (Jason, 2026-10-13). The pane sits in the App group after Appearance, with a bell icon. Its header reads **Notifications**, "Choose when Runner notifies you while the app is in the background." Below the System row, every control is a toggle:

| Section | Row | Subtitle | Default |
| --- | --- | --- | --- |
| Chats | Needs your input | Approval requests, questions or other input. | on |
| Chats | Finishes a turn | The agent is done and waiting for your next message. | on |
| Chats | Fails or exits | Response failed, or the agent process stopped with an error. | on |
| Missions | Asks you a question | A crew member asks you something through the feed. | off |
| Missions | Needs your input | A crew member waits for approval, an answer or input. | off |
| Missions | Goes idle | Every crew member is idle, so the mission has stopped making progress. | off |
| Missions | Fails or exits | A crew member's response fails or its process stops with an error. | off |

- **Independent mission switches** (Jason, 2026-10-13): Missions is one card with four always-visible rows, like Chats. Every mission choice defaults off, and turning one off withdraws only its event notifications. There is no mission master switch; the system controls application-wide permission, banner style, sound and Focus.
- **Needs your input** covers the three waits together, and **Fails or exits** covers the failed turn and the process exit together. The pane offers no finer switches.
- **Storage**: one `NotificationSettings` value inside the app settings in `app_settings.rs`. Its fields are `chats` with `input`, `finished` and `failed`, and `missions` with `question`, `input`, `idle` and `failed`.
- **System row**: a System section at the top of the pane, above the switches, has one row that shows whether the system lets Runner notify (Jason, 2026-10-14, after a live test where the macOS permission request was easy to miss). Runner asks the system when it starts and whenever one of its windows comes to the front, so the row catches up after a visit to System Settings:

  | Permission | Title | Subtitle | Button |
  | --- | --- | --- | --- |
  | Allowed | Notifications are on | Banner style and sound are set in System Settings → Notifications → Runner. | Open System Settings |
  | Off (not allowed yet, the request was declined or ignored, or Runner was turned off) | Notifications are off, with a warning icon | Allow notifications in System Settings → Notifications → Runner. | Open System Settings |
  | Unknown (an unbundled build, or no answer yet) | Allow notifications, banner style and sound | Set by macOS in System Settings → Notifications → Runner. | Open System Settings |

  There is no separate not-set-up state or Allow notifications button. Open System Settings is a bordered secondary button in every state and opens Runner's own page there, `x-apple.systempreferences:com.apple.Notifications-Settings.extension?id=<bundle id>`, with the running app's bundle id, so a development or test build opens its own entry. macOS falls back to the notification list if that bundle has no notification entry; the launch request creates the entry, and human test rebuilds preserve the test bundle identity so they retain it. Windows shows on and off, read from the toast setting; there the subtitles name Settings → System → Notifications, or say "Not allowed in Windows Settings", and the button reads **Open Settings** and opens `ms-settings:notifications`. Banner style, sound and Focus stay the system's settings and are not repeated in Runner.

## Implementation notes

- **Where the code goes**: a notification controller in `crates/runner-app/src/notifications.rs`. It is an app global like `GlobalAppStore`, not a field of `NativeRoot`: every window has its own `NativeRoot`, so a controller on each would post once per open window.
- **App identity**: call `cx.set_app_identity` once at startup. The identifier is `com.wycstudios.runner`, matching the installer shortcut's `AppUserModelID` in `script/windows/runner.iss:71`, and `com.wycstudios.runner-dev` for debug builds so development toasts stay separate. On macOS the bundle supplies the identity.
- **Trigger detection**: a pure function from the previous and next `AppStore::session_statuses` (`app_store.rs:267`, a map of `AgentStatus`), plus the newly appended mission events, to a list of `(session_id, reason)`. It runs on the refreshes that `StoreRefreshKind::for_event` already routes (`session/status`, and `event/appended` for `human_question`), so `runnerd` and the client protocol do not change. Keeping it pure keeps the edge rules unit-testable.
- **Content**: a pure function from the trigger and session to the `SystemNotification` title and body, tested for each row of the content table.
- **Response routing**: `on_system_notification_response` registered during app startup, mapping the tag back to a session or a question and running the View paths above.
- **Permission**: gpui-pre requests permission but never reports it, so `notifications/permission.rs` asks the platform itself. On macOS it reads `UNUserNotificationCenter`'s notification settings, skipped outside an app bundle where that API aborts the process, and treats not-determined and denied as off; on Windows it reads `ToastNotifier::Setting` for the app identity. The answer is kept on the notification controller and the windows are refreshed when it changes.
- **Settings pane**: `SettingsPane::Notifications` in `surfaces/settings_page.rs`, with the route key `notifications`, the label Notifications and `bell.svg`. The lucide bell icon is added to `assets.rs`, and the pane goes into `APP_PANES` after Appearance.

## Non-goals

- A Runner-drawn popup window; the 2026-09-22 design was dropped on 2026-10-13.
- Sound, action buttons, and approving or answering from the notification itself.
- A selectable foreground notification mode; alerts are always background-only (Jason, 2026-10-13).
- An overall Runner or mission master switch or mute; each mission event is independently opt-in, and the system controls application-wide permission (2026-10-13).
- Switches for each session or project, finer switches than the pane's, snoozing, and Time Sensitive notifications that break through Focus.
- Dock bounce and taskbar flashing; the system notification is the alert.
- A Dock badge count, a tray or menu-bar icon, and a view of everything that needs you. That view was [#552](https://github.com/yicheng47/runner/issues/552), closed for the desktop app on 2026-10-08 because it belongs in a mobile companion.
- Notifications while Runner is quit. With **When Runner quits** set to Keep running, `runnerd` keeps the sessions working, but no app is connected to post. Following agents from away from the app is the iOS companion's job ([#836](https://github.com/yicheng47/runner/issues/836)). Whether `runnerd`, which ships inside `Runner.app`, could post with the app's identity is a separate probe, not part of this feature.

## Implementation phases

1. **Design**: the Settings → Notifications pane and the content of each trigger in `design/specs/701-desktop-notifications.pen`, reviewed before any code.
2. **Direct chats**: app identity, the controller, trigger detection, content, response routing, withdrawal and the Notifications pane.
3. **Missions**: slot waits and crashes, `human_question`, and View into the mission workspace.

## Verification

- **Unit tests**:
  - Trigger detection posts only when a state is entered, and the first load sets the baseline.
  - Shell session edges and mission slot finished turns never post; one whole-mission Busy → Idle edge can post when its Goes idle switch is on.
  - Foreground suppression for any Runner route/tab/window, no replay when backgrounded, and withdrawal when a target comes into view.
  - One tag per session.
  - Each switch: nothing posts for a switched-off event.
  - The defaults: chat input, Finished and errors on, all four mission event choices off.
  - The content table, row by row.
  - The System row for each permission state.
- **Checks**: `runner-app` tests and workspace clippy.
- **Live QA on macOS**: notifications need an app bundle, so a plain `make run` build posts nothing.
  - QA builds with `make run` and drives the development build through the `.app` wrapper of the [full smoke-test procedure](../../tests/full-smoke-test.md#macos-native-control-of-the-debug-executable): bundle id `com.wycstudios.runner.smoke-dev`, development data and daemon.
  - The wrapper has its own notification permission, separate from an installed Runner. QA accepts the permission prompt on the first notification.
  - If macOS refuses authorization for the unsigned wrapper, ad-hoc sign it with `codesign --force --sign - "$smoke_bundle"` and record that in the procedure.
  - Do not test with `script/bundle-mac --dev`. Its `Runner.app` uses the production bundle id `com.wycstudios.runner` and release data paths, so it would share notification permission and clicks with an installed Runner.
- **Live QA on Windows**: debug builds post toasts once `set_app_identity` has registered the development identity, so `.\make.cmd run` is enough. Whether a pinned taskbar icon still groups with the running app can only be checked with an installed release, since only the installer gives the shortcut Runner's identity and debug builds use the separate development one.
- **Manual checks on both platforms**:
  - With Runner behind another app, a direct chat that reaches an approval prompt posts one notification without taking focus. Its title is the chat title and its body "Approval needed · <project>".
  - Clicking it brings Runner forward on the right window, tab and pane, including for a session in a second window (⇧⌘N).
  - With Runner in front, another-tab and another-window status edges post nothing. With a notification posted in the background, switching to its target withdraws it. Approving in the terminal withdraws it too.
  - In a split tab of the focused window, an approval wait in the pane without keyboard focus posts nothing.
  - A second trigger for the same chat replaces its notification rather than adding one.
  - With the default switches and Runner in the background, a fresh chat completion posts "Finished · <project>". Turning its switch off withdraws it and suppresses later completions; re-enabling does not replay an observed completion.
  - With Missions → Asks you a question on and Runner in the background, the lead's `ask_human` posts "@lead asked you · <mission>" with the question as the body, and clicking it opens the mission workspace on the feed. With all mission switches off, slots, questions and mission idle post nothing. Each switch enables only its event; Goes idle enables one whole-mission Busy → Idle alert; slot finished turns never post.
  - Turning an event's switch off withdraws its visible notifications, and nothing more posts for it.
  - On a fresh install the System row reads "Notifications are off" with Open System Settings. The button opens the system's notification settings without requesting permission itself. After allowing Runner and returning to it, the row reads "Notifications are on" without a restart.
  - After Don't Allow, or after letting the request go away unanswered, nothing shows and the row reads "Notifications are off" with its warning; allowing Runner in System Settings and coming back to Runner turns the row on and brings notifications back without restarting it.
  - On Windows, turning Runner off in Settings → System → Notifications and coming back to Runner shows "Notifications are off".
  - Open System Settings lands on Runner's own page in Notifications on macOS; Open Settings lands on Settings → System → Notifications on Windows.
  - Quitting Runner withdraws its notifications.
