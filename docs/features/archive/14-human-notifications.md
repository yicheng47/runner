# 14 — System notifications when an agent messages the human

> Tracking issue: [#130](https://github.com/yicheng47/runner/issues/130)

## Motivation

Runner's value proposition is "agents run unattended for hours." For
that to be true in practice, the human can't be tethered to the app
window — they have to be able to walk away, work in another window,
and come back when something needs them. Today there's no surface
that pulls the user back when an agent sends them something.

Two events qualify as "an agent needs the human's attention":

- **Agent posts a message to `@human`** via `runner msg post --to
  human`. Lands as a `kind: message` event with `to: "human"`. Surfaces
  in the workspace feed as a directed message.
- **Agent fires `ask_human`** (`runner signal ask_human --payload
  '{"prompt":"…","choices":["yes","no"]}'`). The router derives a
  `human_question` event that renders as an AskHumanCard with a
  structured choice prompt.

Both are *signals to the human, not the crew*. They should generate a
macOS notification when the human isn't looking at that mission/chat.
Clicking the notification should focus the Runner window and navigate
to the source mission.

This is also the corollary of bug [#128](../) — once the agent
preamble stops mandating a reply on every `human_said`, the remaining
agent-to-human messages are the meaningful ones, and exactly those are
the ones the human should be notified about.

## Scope

### In scope (v1)

- **Add `tauri-plugin-notification` to the workspace.** Wire its
  capabilities into `src-tauri/capabilities/`, request permission on
  first notification (the plugin handles the prompt), gracefully no-op
  if the user denies.
- **Notification triggers** — emit one macOS notification per:
  - New event with `kind: message` and `to: "human"`.
  - New event with `kind: signal` and `type: "human_question"`.
- **Notification body shape:**
  - Title: `@<from> in <mission name>` (or `@<from>` for direct
    chats). For `human_question`, title is `@<asker> asks` to make the
    "you need to act" framing obvious.
  - Body: first ~140 chars of `payload.text` (for messages) or
    `payload.prompt` (for human_question), single-line, no markdown.
- **Click handling.** Clicking the notification:
  - Focuses the Runner window (`set_focus`).
  - Navigates to the mission workspace (`/missions/<id>`) or direct
    chat (`/runners/<handle>/chat/<session_id>`).
  - For `human_question`, scrolls the AskHumanCard into view.
- **Suppression rules.** Notifications are not fired when **all** of:
  - The app has OS focus (any Runner window is the foreground app).
  - The window currently showing the source mission/chat is also the
    focused window.
  - That window is on the feed tab (not the PTY tab) — the user is
    actively watching the feed and will see the event land.
- **Single global on/off toggle.** New "Notifications" row in the
  Settings modal. Default: on after first install (the plugin still
  requires per-user OS permission grant, so "on" is gated by the OS
  permission too).

### Out of scope (deferred)

- **Per-mission / per-runner mute.** "Mute mission X" or "mute @bot."
  Real feature once notifications become a daily presence; v1 ships
  with global on/off only.
- **Notification sound customization.** The default macOS notification
  sound is fine. Custom sounds add a tiny audio asset pipeline.
- **In-app toast surface.** When the app *is* focused but the user is
  on a different mission, we currently rely on the sidebar's unread
  count (an existing affordance) to flag the inbound. A floating
  in-app toast that pops in the corner would be nicer; out of scope
  for v1.
- **Notification grouping / batching.** macOS auto-groups
  notifications from the same app reasonably well; no need for
  app-side batching in v1. If a chatty agent floods the human, we
  surface it as a UX bug, not a notification feature.
- **Rich notification actions.** macOS lets you attach quick-reply
  actions ("Yes" / "No" buttons from the notification). Tempting for
  `human_question` events, but the notification plugin doesn't expose
  this cleanly and the AskHumanCard's structured-choice flow already
  belongs in the app window. v2.
- **Cross-window notification routing** (spec 12). When multi-window
  ships, clicking a notification should route to whichever window is
  currently primary for the source mission, or open a new one if
  none. v1 ships before spec 12; the click handler can be naive
  ("focus main window, navigate") and gets refined when spec 12
  lands.

### Key decisions

1. **The event log is the trigger, not the IPC layer.** A backend
   subscriber tails the same NDJSON event stream the workspace UI
   reads from. When a qualifying event lands, the backend fires the
   notification. This means the rule "notify on message-to-human"
   lives in one place and applies to events from any source —
   agent-emitted, future router-synthesized, replay on bus restart.
   No risk of forgetting to notify on some new code path.
2. **Suppression check happens in the backend.** The backend tracks
   "which window has OS focus" and "what subject is that window
   currently viewing" (this state is already needed for spec 12 and
   gets bootstrapped here even if 12 isn't shipped yet). When deciding
   whether to fire a notification, the backend consults the registry.
   Frontend-side suppression would require every window to listen to
   every event just to know whether to *not* show a notification —
   wasteful and racy.
3. **Don't notify on broadcast messages.** A `kind: message` event
   with `to: null` is a crew-internal broadcast — useful to other
   agents, not to the human. Notifying on these would generate noise
   any time the crew chats among itself. The rule is strictly
   `to == "human"`.
4. **Notify even when other Runner windows are focused on something
   else.** If the user has window A on mission 1 focused, and a
   `human_question` lands in mission 2, fire the notification. The
   app-focus heuristic isn't enough — *what* the user is looking at
   matters. Suppression requires "looking at this specific subject,"
   not just "app is foreground."
5. **AskHumanCard remains the source of truth for human-question
   resolution.** The notification is a pointer; the structured
   choice is made inside the app. No notification-action shortcuts in
   v1 (see deferred).

## Implementation phases

### Phase 1 — plugin + capability

- Add `tauri-plugin-notification` to `src-tauri/Cargo.toml` and
  initialize it in `lib.rs::run`.
- Wire capabilities (`src-tauri/capabilities/default.json` or
  similar) to allow `notification:default`.
- Add `@tauri-apps/plugin-notification` to `package.json` so the
  frontend can read permission state if we ever need to surface
  "notifications are blocked, fix in System Settings" in the UI.

### Phase 2 — backend subscriber

- New module `src-tauri/src/notifications.rs`:
  - Subscribes to the same per-mission `BusEvents` stream the
    workspace UI uses.
  - For each event, applies the trigger rules (`kind: message, to:
    "human"` or `kind: signal, type: "human_question"`).
  - For each direct-chat session, subscribes similarly.
- Per-event flow:
  1. Apply trigger rule. If no match, drop.
  2. Apply suppression rule — query the window registry (spec 12 v1
     stub, just "is the main window focused on this subject"). If
     suppressed, drop.
  3. Compose title + body. Call
     `tauri::plugin::notification::Notification::builder()`.
  4. On click, emit a Tauri event `notification_clicked { subject,
     event_id }` that the frontend listens for and uses to navigate.
- Bus-level subscription means notifications work for all sessions
  the app has loaded, not just the one currently in the foreground.

### Phase 3 — window-focus tracking (spec-12 lite)

- Even without full multi-window, we need to know "is the app
  foreground" and "what subject is the visible window on" for
  suppression. Spec 12 calls this out as Phase 1 (`WindowRegistry`);
  ship the same registry here, scoped to the single main window for
  v1. When spec 12 lands, the registry generalizes naturally to N
  windows.
- Frontend reports its subject on route changes via
  `window_report_subject(subject)` (same command spec 12 will use).
- Frontend reports its focus state via Tauri's `WindowEvent::Focused`
  → backend updates registry.

### Phase 4 — frontend wiring

- New `src/lib/notifications.ts`: listens for the backend's
  `notification_clicked` event, calls the router's `navigate(...)` to
  the target subject. For `human_question`, also scrolls the
  AskHumanCard into view (use a `?focus=<event_id>` query param the
  workspace reads on mount).
- New row in `SettingsModal.tsx`: "System notifications" toggle.
  Persists to the existing settings store. Backend reads on each
  notification attempt; off → drop silently.

### Phase 5 — verification

- Backend unit tests:
  - Trigger rule: `kind: message, to: "human"` fires; `to: null`
    doesn't; `kind: signal, type: "runner_status"` doesn't.
  - Suppression rule with mock registry: app-foreground + subject
    match → suppressed; else → fired.
- Manual smoke:
  1. Grant notification permission on first run.
  2. From mission A, have an agent run `runner msg post --to human
     "test"`. Observe macOS notification. Click → window focuses,
     navigates to mission A's feed.
  3. With mission A in foreground and on the feed tab, have the
     agent post another message. No notification.
  4. Switch to mission B's workspace; an agent in mission A fires
     `ask_human`. Notification fires; click → navigate to mission A,
     AskHumanCard is scrolled into view.
  5. Toggle "System notifications" off in Settings. Repeat (2) → no
     notification fires, no permission prompt re-shown.

## Verification

- [ ] macOS notification fires on `kind: message, to: "human"`.
- [ ] macOS notification fires on `kind: signal, type: "human_question"`.
- [ ] No notification on broadcast messages (`to: null`) or any other
      event types.
- [ ] No notification when the app is foreground and the source
      subject is the currently-visible one.
- [ ] Click on notification focuses the Runner window and navigates
      to the source mission / chat; for `human_question`, scrolls the
      card into view.
- [ ] Settings toggle disables all notifications without re-prompting
      OS permission.
- [ ] No notifications fire while the workspace is replaying events
      on session reattach (only live appends).
- [ ] `cargo test --workspace` and `pnpm exec tsc --noEmit` clean.
- [ ] First-run permission prompt is well-timed — at the first
      attempted notification, not at app launch.
