# Multi-window Frontend (Arc-style duplicate-subject overlay)

## Status

In progress for issue [#122](https://github.com/yicheng47/runner/issues/122). Implements feature spec [docs/features/12-multi-window.md](../features/12-multi-window.md).

## Problem

Runner ships as a single Tauri window (`src-tauri/tauri.conf.json` declares exactly one, label `main`). For an editor whose unit of work is a long-running mission with sub-conversations, that's a constraint: the user can't park mission A on one monitor while mission B runs on another, or watch a feed in one window while tailing a PTY in another.

The backend is already single-source-of-truth: one process, one `SessionManager`, one `RouterRegistry`, one `BusRegistry`, and event emission via `app.emit(...)` already broadcasts to every webview (`event_bus::TauriBusEvents`, `session::manager::TauriSessionEvents`). The missing pieces are entirely about *coordination*: spawning additional webviews, tracking which subject (mission / direct-chat) each window is looking at, and gating PTY attach so two windows never write to the same stdin.

## Goals

- Spawn additional webview windows via `Cmd+N`, a `File → New Window` menu item, an "Open in New Window" sidebar context action, and a `window_open` command.
- Each window owns an independent `BrowserRouter` history and reports its current subject (mission id / direct-chat session id) to the backend on route change.
- A backend `WindowRegistry` maps window label → subject + `focused_at`, and broadcasts `window_focus_map` after every mutation.
- Arc-style overlay: when two windows share a subject, the more-recently-focused window is **primary**; the other(s) render an overlay with "Focus that window" → `set_focus()` on the primary.
- **PTY / xterm mounts only in the primary**: terminal components and stdin-capable session UI do not mount while a window is secondary; re-attach on flip back to primary. `mission_attach` remains an idempotent backend lifecycle call unless implementation proves it must be primary-only.
- Closing a secondary unregisters/destroys it; closing the primary promotes the surviving next-most-recent focused window for that subject. The `main` window keeps its existing hide-on-close behavior.

## Non-Goals (v1, deferred)

- Window restoration across app restart (pairs with spec 10 / mission-session persistence).
- Per-window settings (zoom / font / theme remain app-global).
- Tab-tearing, shared-PTY render across windows, multi-instance / multi-process.
- Targeted `emit_to(label, ...)`. Broadcast `emit` stays; each window filters by its own subject (already the case for `event/appended`, `session/output`, etc.).

## Key Constraints Discovered

1. **Capabilities are scoped to `["main"]`.** `src-tauri/capabilities/default.json` lists `"windows": ["main"]`, so a freshly spawned `window-<ulid>` would have **zero** permissions — it couldn't `invoke` a command or `listen` to an event. This must change to a glob (`["main", "window-*"]`). Because windows are created from Rust (`WebviewWindowBuilder`), we do **not** need the JS-side `core:webview:allow-create-webview-window` permission. `core:window:allow-set-focus` is already present, which `window_focus_other` needs.

2. **`app_ready` shows only `main`.** `commands::app::show_main_window` hard-codes `get_webview_window("main")`. Secondary windows start hidden to avoid the white-flash, so the show-after-first-paint command must show the *calling* window. Generalize `app_ready` to take the invoking `tauri::WebviewWindow` and show that.

3. **Frontend is `BrowserRouter` (HTML5 history), not hash router.** A new window loads `index.html`; a deep path like `/missions/<id>` won't resolve through Tauri's asset protocol in release builds. Pass the initial route as a URL **hash fragment** (`index.html#/missions/<id>`) and have a small bootstrap read `location.hash` on mount, `navigate()` to it, and clear the hash. This matches the spec and sidesteps deep-link asset resolution.

4. **`main` close is special-cased.** `lib.rs::run`'s `RunEvent` handler intercepts `CloseRequested` for `label == "main"`, prevents close, and hides. Secondary windows must be allowed to actually close (destroy). Registry cleanup keys off `WindowEvent::Destroyed`, not `CloseRequested`, so it fires for both the genuine close of a secondary and any future real close.

## Proposed Backend Design

### Phase 1 — `WindowRegistry` module (`src-tauri/src/windows.rs`)

New module, mirroring the `Mutex<HashMap<...>>` shape used elsewhere in the crate.

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

`Subject` derives `Serialize`/`Deserialize`/`Clone`/`PartialEq`. Serialize as a tagged enum so the frontend gets `{ "type": "Mission", "value": "..." }` or similar — pick the serde tag shape and mirror it in `src/lib/types.ts`. `WindowEntry` is `Serialize` for the broadcast payload.

Methods:

- `register(label)` — insert with `subject: None`, `focused_at: now()`.
- `unregister(label)` — remove.
- `set_subject(label, Option<Subject>)` — update subject; leave `focused_at` untouched (subject change ≠ focus change).
- `mark_focused(label)` — bump `focused_at = now()`.
- `snapshot() -> Vec<WindowEntry>` — for the broadcast payload and `window_list_subjects`.
- `primary_for(&Subject) -> Option<String>` — among windows holding this subject, the one with the **max** `focused_at` is primary.

Primary invariant: **most-recently-focused window owns the subject**. When a window is focused, `focused_at = now()`. A window is secondary when another window holds the same subject with a later `focused_at`. When the primary closes, the next-most-recent survivor becomes primary.

Add to `AppState` (`lib.rs`): `pub windows: Arc<windows::WindowRegistry>`. Construct in `setup`, register `"main"` immediately, and `.manage` it as part of `AppState` (no separate `.manage`).

### Phase 1 (cont.) — Tauri window lifecycle hooks

In `lib.rs::run`'s `RunEvent` match, extend window handling:

- `WindowEvent::Focused(true)` for any label → `registry.mark_focused(label)` then broadcast.
- `WindowEvent::Destroyed` for any label → `registry.unregister(label)` then broadcast.
- `WindowEvent::CloseRequested` for `label == "main"` → unchanged (prevent + hide). App quit remains the explicit Quit / `Cmd+Q` path, not close-main-window.
- `WindowEvent::CloseRequested` for other labels → let it proceed (default destroy); the subsequent `Destroyed` handles unregister.

Broadcast helper: `fn broadcast_focus_map(app: &AppHandle)` → `app.emit("window_focus_map", registry.snapshot())`. Called from every mutation (hooks + commands).

### Phase 2 — Tauri window-opening helper + commands (`src-tauri/src/commands/window.rs`, new module)

Register in `commands/mod.rs` (`pub mod window;`) and add all four to the `generate_handler!` list in `lib.rs`.

- Shared helper `open_window(app: &AppHandle, state: &AppState, initial_route: Option<String>, position: Option<(i32, i32)>) -> Result<String>` — used by both the Tauri command and the Rust menu handler. Keep the window-building logic out of the command wrapper so `on_menu_event` can call the same path via `app.state::<AppState>()`.
- `window_open(app, state, initial_route: Option<String>, position: Option<(i32, i32)>) -> Result<String>` — thin command wrapper around the shared helper.
  - Generate label `window-<ulid>` (ULID dep already present).
  - `WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(url.into()))` where `url` is `"index.html"` or `"index.html#<initial_route>"`.
  - Mirror `main`'s chrome via builder methods: `.title("Runner")`, `.inner_size(1440.0, 900.0)`, `.title_bar_style(TitleBarStyle::Overlay)`, `.hidden_title(true)`, `.traffic_light_position(Position)`, `.accept_first_mouse(true)`, `.visible(false)` (shown by the generalized `app_ready` after first paint). Apply `position` if provided, else a small cascade offset from the focused window.
  - `registry.register(&label)`, broadcast, return label.
- `window_focus_other(app, label: String) -> Result<()>` — `app.get_webview_window(&label).map(|w| w.set_focus())`.
- `window_report_subject(window: tauri::WebviewWindow, state, subject: Option<Subject>) -> Result<()>` — resolve label from `window.label()`, `registry.set_subject(label, subject)`, broadcast.
- `window_list_subjects(state) -> Result<Vec<WindowEntry>>` — `registry.snapshot()` for hydrate-on-mount.

Generalize `commands::app::app_ready` to `app_ready(window: tauri::WebviewWindow)` and show/focus the **calling** window instead of hard-coded `"main"`. `show_main_window` stays for the macOS `Reopen` path.

### Phase 2 (cont.) — capabilities

Edit `src-tauri/capabilities/default.json`: `"windows": ["main", "window-*"]`. No new permissions needed (creation is Rust-side; `set-focus`, event listen, and command invoke are already covered by the existing list + `core:default`).

### Backend tests

Unit tests in `windows.rs`:

- `register` / `unregister` round-trip.
- `set_subject` updates subject without touching `focused_at`.
- `mark_focused` bumps timestamp; `primary_for` returns the most-recently-focused holder.
- Promotion: two windows on a subject, primary unregistered → `primary_for` returns the survivor.
- `None` subjects never count as a duplicate (no overlay for empty windows).

These are pure registry tests (no Tauri runtime), so they run under `cargo test --workspace` like the existing command unit tests.

## Proposed Frontend Design

### Phase 3 — coordination layer (`src/lib/windowFocus.ts`)

- `useCurrentWindowLabel()` — caches `getCurrentWindow().label` at module load (from `@tauri-apps/api/window`).
- `useWindowFocus()` — subscribes to `window_focus_map` (via `listen`) and exposes the latest `WindowEntry[]`; hydrates once on mount via `api.window.listSubjects()` so it doesn't wait for the first broadcast.
- `reportSubject(subject: Subject | null)` — debounced wrapper over `window_report_subject`.
- Derived helper `isSecondaryFor(map, myLabel, subject)` — true when another entry holds the same subject with a later `focused_at` than mine. Returns the primary's label for the overlay subtitle / focus button.
- `api.window` block added to `src/lib/api.ts`: `open(initialRoute?, position?)`, `focusOther(label)`, `reportSubject(subject)`, `listSubjects()`. `Subject` / `WindowEntry` types added to `src/lib/types.ts`.

### Phase 3 (cont.) — bootstrap initial route

In `App.tsx` (inside the `BrowserRouter`), a tiny `<InitialRouteBootstrap>` effect: on first mount, if `location.hash` is non-empty, `navigate(hash.slice(1), { replace: true })` and clear the hash. Runs once per window. (Alternatively in `main.tsx` before render — but doing it inside the router via `useNavigate` is cleaner.)

### Phase 3 (cont.) — wire subjects + gate attach

- `MissionWorkspace` (`src/pages/MissionWorkspace.tsx`): `reportSubject({ Mission: id })` on mount, `reportSubject(null)` on unmount. Compute `isSecondary` from `useWindowFocus()`. The data-loading portion of the mount effect may still fetch mission/session/event state, and `api.mission.attach(id)` may remain safe/idempotent; the hard gate is terminal ownership. While `isSecondary`, do not auto-populate `openTabs`, do not render `RunnerTerminal`, and do not send stdin/resize/start requests. On flip secondary→primary, attach/mount terminals normally; on flip primary→secondary mid-session, unmount terminals, clear terminal refs, and avoid leaving hidden PTYs mounted. Render `<DuplicateSubjectOverlay>` over the main content area when secondary.
- `RunnerChat` (`src/pages/RunnerChat.tsx`): `reportSubject({ DirectChat: sessionId })` / `null`; same `isSecondary` gating around the `RunnerTerminal` map and `session_start_direct` / resize path. On flip primary→secondary, unmount the terminal map and reset the local attach/start refs needed for a clean reattach when it becomes primary again. Render the overlay.
- Other route pages report `null` on mount (or rely on the previous subject being cleared on the unmount of Mission/Chat). Simplest: a shared `useReportSubject(subject)` hook that reports on mount and clears on unmount; non-subject pages call `useReportSubject(null)`.

### Phase 3 (cont.) — `<DuplicateSubjectOverlay>` (`src/components/DuplicateSubjectOverlay.tsx`)

Absolute-positioned card over the content area. Title "Open in another window", subtitle showing the primary window's label / workspace, primary button "Focus that window" → `api.window.focusOther(primaryLabel)`, secondary "Stay here" link that hides the overlay for the current view (no persisted dismiss flag — reappears on navigate-away-and-back, per spec decision 6). Styled to match existing modal/card components (reuse Tailwind tokens from e.g. `SettingsModal`).

### Phase 4 — new-window affordances + shortcut swap

**Shortcut convention (browser/terminal standard): `Cmd+T` = new chat, `Cmd+N` = new window.** Today `Cmd+N` opens the Start Chat modal via the Sidebar keydown handler (`src/components/Sidebar.tsx:249`, `setCreatingChat(true)`, hint `⌘N` at line 913). Rebind:

- **`Cmd+T` → new chat**: in the Sidebar `onKey` handler change the `e.key === "n"` branch to `"t"` (keep `setCreatingChat(true)`); update the `⌘N` hint on `NewChatNavRow` to `⌘T` and the comment at `Sidebar.tsx:224`. Remove `n` from the JS handler entirely so it doesn't double-fire with the menu accelerator below.
- **`Cmd+N` → new window**: add `File → New Window` to `build_menu` in `lib.rs` with a `Cmd+N` (`CmdOrCtrl+N`) accelerator. macOS adds a `File` submenu (currently absent); route the menu event in `on_menu_event` to the shared `open_window(..., None, None)` helper. Owning the shortcut at the OS/menu level (rather than a JS keydown handler) is cleaner and is why the JS `n` branch is removed. Non-macOS: add `File → New Window` to the existing menu.

> Order note: both changes land together in this phase, so `Cmd+N` is never dead — it's only repurposed once `window_open` (Phase 2) exists.

- **Sidebar context menus**: extend the existing per-row context menus (mission rows, session rows — state already exists at `sessionMenu`) with "Open in New Window" → `api.window.open("/missions/<id>")` or `api.window.open("/chats/<sessionId>")`.
- **Empty state**: a `Cmd+N` window with no initial route lands on `/runners` (existing default), `subject: None`, no overlay.

### Frontend checks

`pnpm exec tsc --noEmit` and `pnpm run lint` clean.

## Phase 5 — Verification & smoke

Backend: `cargo test --workspace` (registry unit tests + existing suite).

Manual smoke (from spec §Phase 5):

1. `Cmd+N` opens a second functional window; both navigate independently.
2. Window A opens mission X, window B opens mission X → B is primary, A shows overlay. "Focus that window" from A brings B forward.
3. Focus A → A becomes primary, B picks up the overlay (flip works both ways).
4. In secondary B, navigate to a different mission → both overlays clear (A alone on X).
5. Same direct chat in two windows → same rules; exactly one xterm mounted, PTY input lands once.
6. With two windows back on X, close the primary for X → survivor's overlay clears, becomes primary.
7. Close secondary windows → they unregister/destroy; close `main` → app hides as it does today. Quit via `Cmd+Q` / app menu → only `main` returns on reopen (no restoration; deferred).

## Relevant Code

- `src-tauri/src/lib.rs:32-53` — `AppState` (add `windows`).
- `src-tauri/src/lib.rs:106-244` — `setup` (construct + register `main`).
- `src-tauri/src/lib.rs:302-342` — `RunEvent` handler (Focused / Destroyed / CloseRequested hooks).
- `src-tauri/src/lib.rs:246-299` — `generate_handler!` (register window commands).
- `src-tauri/src/lib.rs:414-469` — `build_menu` (File → New Window).
- `src-tauri/src/commands/mod.rs` — add `pub mod window;`.
- `src-tauri/src/commands/app.rs:9-21` — `app_ready` / `show_main_window` (generalize to calling window).
- `src-tauri/src/commands/mission.rs:906-1033` — `mission_attach` (kept idempotent unless implementation proves it must be primary-only).
- `src-tauri/capabilities/default.json` — `windows` glob.
- `src/App.tsx:73-91` — router + initial-route bootstrap.
- `src/lib/api.ts`, `src/lib/types.ts` — `api.window`, `Subject`, `WindowEntry`.
- `src/pages/MissionWorkspace.tsx:143-257`, `src/pages/RunnerChat.tsx` — report subject + gate attach + overlay.
- `src/components/RunnerTerminal.tsx` — mounted/unmounted by the gating logic (no internal change expected).
- `src/components/Sidebar.tsx:167-436` — context-menu "Open in New Window".

## Implementation Order

1. Backend registry + lifecycle hooks + `AppState` wiring + unit tests (Phase 1).
2. Backend commands + capabilities + generalized `app_ready` (Phase 2).
3. Frontend `windowFocus.ts` + types/api + initial-route bootstrap (Phase 3a).
4. Overlay component + subject reporting + attach gating in Mission/Chat (Phase 3b).
5. Menu + sidebar affordances (Phase 4).
6. Typecheck/lint/tests + manual smoke (Phase 5).
