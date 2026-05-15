# 12 — Multi-window frontend

> Tracking issue: [#122](https://github.com/yicheng47/runner/issues/122)

## Motivation

Runner ships as a single Tauri window today (`src-tauri/tauri.conf.json`
declares exactly one). For an editor whose primary unit of work is a
long-running mission with many sub-conversations, that's a constraint:
the user can't have mission A on a left monitor while watching mission
B's lead stream on the right, can't park a direct chat next to the
mission it spawned from, can't open the same workspace twice to keep one
focused on the feed while the other tails a PTY.

Arc browser handles this gracefully with a soft rule: multiple windows
are first-class, but if two windows look at the same tab, only one
"owns" it — the other shows a small overlay pointing back to the owner.
That's the model this spec adopts. The backend stays single-source-of-
truth (one process, one session manager, one router); the frontend gains
the ability to spawn additional webview windows, each with its own
routing state and its own active mission/chat.

Power-user value is the obvious win. The deeper reason: spec 10
(mission-session persistence) made it so agents survive an app quit. If
agents are durable but the window topology is brittle, the user can't
build a stable physical workspace around them. Multi-window completes
that story.

## Scope

### In scope (v1)

- **Spawn additional Tauri webview windows.** Each window mounts the
  same React app at the same dev/build URL, but with a unique window
  label (`main`, `window-<ulid>`). The window has its own
  `BrowserRouter` history and can navigate to any route independently.
- **New-window affordances.**
  - Menu / shortcut: `File → New Window` (`Cmd+N`).
  - Sidebar context affordance: right-click (or `Cmd+click`) on a
    mission row or direct-chat row → "Open in New Window."
  - Programmatic: a `window_open` Tauri command takes an optional
    `initial_route` and a position hint.
- **Per-window route tracking.** The frontend reports its current
  "primary subject" (mission id or session id) to the backend whenever
  the route changes. Carrying the full URL is overkill; the
  coordination problem is at the mission/chat level.
- **Cross-window coordination registry.** Backend keeps
  `HashMap<WindowLabel, Option<Subject>>` where `Subject` is
  `Mission(id)` or `DirectChat(session_id)`. On any change, it broadcasts
  a `window_focus_map` event so every window has a consistent picture.
- **Arc-style overlay.** If a window's current subject is also held by
  another window with an *earlier* `focused_at` timestamp, this window
  is the **secondary**. The mission/chat view renders an overlay over
  the main content area:
  - Title: "Open in another window"
  - Subtitle: window label or workspace name of the primary
  - Primary action button: "Focus that window" → invokes
    `window_focus_other { label }` on the backend, which calls
    `set_focus()` on the target window.
  - Dismiss: navigate this window away from the duplicated subject
    (e.g. to `/runners` empty state, or back via router history).
- **No PTY / xterm mount on the secondary.** xterm has DOM state and
  consumes stdin; double-mounting against one session creates input
  races. The overlay is what gates the mount: the workspace component
  skips `mission_attach` / `session_attach` while the overlay is up,
  and unmounts the terminal if the window flips from primary to
  secondary mid-session.
- **Window close behavior.** Closing a non-last window just unregisters
  it from the focus map; the backend keeps running. Closing the *last*
  window quits the app (today's behavior). If the primary window for a
  subject closes, the secondary becomes primary automatically (the
  earliest `focused_at` among surviving windows wins).

### Out of scope (deferred)

- **Window restoration across app restart.** Persist window list +
  positions + per-window last subject to the settings DB, restore on
  app start. Real product win — pairs with spec 10's mission
  persistence so the entire workspace survives a relaunch — but not
  required for the multi-window mechanic to land. Track as a follow-up.
- **Per-window settings.** Today, settings (zoom, terminal font, theme)
  are app-global. Some are already per-webview at the OS level (zoom
  via `getCurrentWebview()`), but the *stored* setting is global. Out
  of scope to fan settings out per-window.
- **Drag-a-tab-to-make-a-window.** Browser-style tab tearing. Cool, not
  needed when the sidebar already lists every subject and the new-window
  affordance is one click away.
- **Sharing one PTY render across windows.** True simultaneous render
  with synchronized cursor / scrollback. Out — the overlay sidesteps
  this entirely.
- **Coordinated focus on cross-window navigation.** When window A
  navigates *away* from mission X, the spec doesn't auto-focus or
  surface a notification to window B (which is now alone on X). The
  focus map updates; window B silently becomes / stays primary.
- **Multi-instance (multiple OS processes).** Out. Single backend
  process, multiple windows.

### Key decisions

1. **Subject granularity is mission-id / session-id, not full URL.**
   Two windows on the same mission but on different inner tabs (feed
   vs. terminal) are still "looking at the same mission" — the overlay
   should fire. Tracking by URL would let the user double-open the same
   mission feed and never notice. Mission-id keeps the rule sharp.
2. **"Primary" is by `focused_at`, not creation order.** When the user
   focuses a window (either by clicking it or by bringing it forward
   via the overlay's "Focus that window" button), it becomes primary
   for whatever subject it's on. This matches Arc's intuition that the
   window you just touched is the one that "owns" the work.
3. **Backend is the registry, not the windows.** Each window reports
   its subject + focus events to the backend; the backend computes the
   map and fans it out. Peer-to-peer coordination via the Tauri event
   bus would race on window-spawn and need conflict resolution. Central
   registry is one mutex, one source of truth.
4. **The overlay gates the mount, not just the display.** A secondary
   window doesn't merely *hide* the terminal — it never calls
   `session_attach` / `mission_attach` in the first place. This is what
   keeps stdin / PTY input single-writer and what makes `mission_attach`
   idempotent contract still hold (one attached window per mission).
5. **`emit` stays broadcast.** Today `app.emit("…", payload)` reaches
   every webview. That's fine: each window's frontend filters by its
   own current subject. No need to switch to `emit_to(label, …)` for
   v1 — the wasted bytes are negligible. Targeted emit becomes useful
   later for per-window dialogs / notifications.
6. **The secondary is allowed to navigate.** Nothing locks the
   secondary's UI — sidebar still works, the user can move it off the
   duplicated subject any time. The overlay is a soft hint, not a modal.

## Implementation phases

### Phase 1 — backend window registry

- New module `src-tauri/src/windows.rs`:
  ```rust
  pub enum Subject {
      Mission(String),
      DirectChat(String), // session_id
  }
  pub struct WindowEntry {
      pub label: String,
      pub subject: Option<Subject>,
      pub focused_at: chrono::DateTime<Utc>,
  }
  pub struct WindowRegistry { /* Mutex<HashMap<String, WindowEntry>> */ }
  ```
- Methods: `register(label)`, `unregister(label)`, `set_subject(label,
  subject)`, `mark_focused(label)`, `snapshot() -> Vec<WindowEntry>`,
  `primary_for(subject) -> Option<&str>`.
- Hook into Tauri's `WindowEvent::CloseRequested` / `Focused(true)` in
  `src-tauri/src/lib.rs::run` to keep registry state honest.
- Backend emits `window_focus_map` (a single broadcast event with the
  current snapshot) after every mutation.

### Phase 2 — Tauri commands for windows

- `window_open(initial_route: Option<String>, position: Option<(i32, i32)>) -> String`
  — returns the new window's label. Uses `WebviewWindowBuilder::new(…)`
  with a unique label (`window-<ulid>`), points at the same URL as the
  main window plus a hash fragment carrying `initial_route` if provided.
- `window_focus_other(label: String) -> ()` — calls
  `app.get_webview_window(&label).set_focus()`.
- `window_report_subject(subject: Option<Subject>) -> ()` — frontend
  calls this on route change; resolves `WindowLabel` from the
  invoking webview (`tauri::Webview::label()`), updates the registry.
- `window_list_subjects() -> Vec<WindowEntry>` — frontend can hydrate
  on mount instead of waiting for the next broadcast.

### Phase 3 — frontend coordination

- `src/lib/windowFocus.ts`:
  - `subscribeToFocusMap(callback)` — listens for `window_focus_map`
    events and exposes the latest snapshot via a hook
    `useWindowFocus()`.
  - `reportSubject(subject)` — debounced wrapper around
    `window_report_subject`.
  - `useCurrentWindowLabel()` — caches `getCurrentWindow().label`
    once at module load.
- Wire `reportSubject` from the route components: `MissionWorkspace`
  reports `Mission(id)` on mount and `null` on unmount;
  `RunnerChat` reports `DirectChat(session_id)`. Other pages report
  `null`.
- A new `<DuplicateSubjectOverlay>` component:
  - Reads the focus map + this window's label and current subject.
  - If another window has the same subject with an earlier
    `focused_at`, render an absolute-positioned card over the main
    content area: title, subtitle, "Focus that window" button,
    secondary "Stay here" link.
  - When dismissed via "Stay here," the user is intentionally creating
    a duplicate view — the overlay should reappear if they navigate away
    and back. (Don't persist a per-subject dismiss flag; keep it simple.)
- `MissionWorkspace` and `RunnerChat` check `isSecondary` from the
  focus map and short-circuit `mission_attach` / `session_attach` calls
  + terminal mount while it's true. They re-attach on flip back to
  primary.

### Phase 4 — new-window affordances

- Menu item: add `File → New Window` via Tauri's menu builder. Shortcut
  `Cmd+N` (macOS) / `Ctrl+N`. Calls `window_open(None, None)`.
- Sidebar: right-click on a mission row or direct-chat row →
  contextual "Open in New Window." Calls
  `window_open(Some("/missions/<id>"), None)` (or the chat route).
- Empty-state behavior: a freshly opened window with no initial route
  lands on `/runners` (the existing default). The window registry
  shows `subject: None`, so no overlay fires.

### Phase 5 — verification + smoke

- Backend tests: unit tests for `WindowRegistry` covering register /
  unregister / set_subject / focus / primary_for, including the
  "earliest focused_at wins" rule.
- Frontend manual smoke:
  1. Open Runner. `Cmd+N` opens a second window. Both windows are
     functional, can navigate independently.
  2. Window A opens mission X. Window B opens mission X. Window B
     shows the overlay (it's the more recent focus). Click "Focus
     that window" — window A comes forward, window B keeps showing
     the overlay.
  3. Click into window B (focus it). Now B is primary, A's mission
     view picks up the overlay. (Focus flip works both directions.)
  4. In window A (secondary now), navigate to a different mission.
     A's overlay disappears; B's overlay disappears (B is alone on
     mission X again, becomes primary).
  5. Open the same direct chat in two windows. Same overlay rules.
     Confirm only one window's xterm is mounted at a time and the
     PTY input lands once.
  6. Close window A (the primary for mission X). B's overlay
     disappears, B becomes primary.
  7. Quit the app (close the last window). On reopen, only the main
     window comes back (window restoration is out of scope for v1).

## Verification

- [ ] `Cmd+N` opens a new window that mounts the same app and routes
      independently.
- [ ] "Open in New Window" on a sidebar item opens a window already
      navigated to that mission/chat.
- [ ] Two windows on the same mission: the more-recently-focused one
      is primary; the other shows the duplicate-subject overlay.
- [ ] The overlay's "Focus that window" button brings the primary to
      the front.
- [ ] PTY / xterm mounts only in the primary window for any given
      session. Input lands in exactly one stream.
- [ ] Closing the primary promotes the secondary to primary; overlay
      disappears.
- [ ] Closing the last window quits the app. (Today's behavior, not
      regressed.)
- [ ] Backend `window_focus_map` broadcasts after every mutation; the
      frontend reflects it without polling.
- [ ] `cargo test --workspace` and `pnpm exec tsc --noEmit` clean.
- [ ] Window restoration on app restart is *not* claimed (deferred).
