# Restore main-window geometry across app restarts

## Status

Planned. Tracks issue [#271](https://github.com/yicheng47/runner/issues/271) (feature, issue-only by request — no spec file).

## Problem

Every launch opens the main window at the hard-coded config geometry (`src-tauri/tauri.conf.json`: 1440×900, OS-placed). Nothing persists where the user last put or sized the window, so anyone who works with Runner on a specific monitor or screen-half re-places it on every restart.

## Investigation: `tauri-plugin-window-state` v2.4.1

The issue asked to evaluate the official plugin before hand-rolling. Verdict: **use it** — its mechanics line up with Runner's launch flow exactly, and hand-rolling would re-derive all of it. Findings from the plugin source (v2.4.1):

- **Restore happens in `on_window_ready`** — after native window creation, before the webview loads React. The main window is still hidden at that point (`visible: false` + show-after-first-paint via `app_ready`), so geometry is applied with zero flicker: the window first becomes visible already at its restored position/size.
- **Persistence model**: an in-memory cache per window label, updated on `Moved`/`Resized`/`CloseRequested` events, written to disk as pretty-printed JSON only on `RunEvent::Exit`. File lives at `app_config_dir()/.window-state.json` (macOS: `~/Library/Application Support/com.wycstudios.runner/`); only the filename is configurable, not the directory.
- **Off-screen recovery is built in**: on restore, the saved position is applied only if a corner of the saved rect intersects an available monitor — otherwise the OS picks placement (size still restores). Covers the issue's monitor-unplugged edge case with no extra code.
- **Selective restore via `StateFlags`**: SIZE, POSITION, MAXIMIZED, VISIBLE, DECORATIONS, FULLSCREEN are independently opt-in.
- **Exclusion hooks**: `with_filter(|label| ...) -> bool` runs before restore *and* before a window is inserted into the tracking cache, so filtered windows never touch the state file.

Interplay checks against Runner's window lifecycle — all clean:

- Main hides on close rather than destroying (`lib.rs:341` calls `prevent_close()` + hide), so at `RunEvent::Exit` the window still exists and the final save reads live geometry. The plugin's own `CloseRequested` listener also still fires (listeners are independent of our `prevent_close`) and harmlessly refreshes the cache.
- Plugin save runs on `RunEvent::Exit`, which fires after our `ExitRequested` session-teardown handler — no ordering conflict.
- macOS overlay title bar keeps the window decorated, so the plugin's `is_maximized` workaround for undecorated windows (tauri#5812) never mis-triggers.

## Key Decisions

1. **Use the plugin, Rust-side only.** No npm guest package, no capability grants — the plugin registers three invoke commands, but nothing in `capabilities/default.json` will allow them, so the webview can't call them. All behavior comes from builder configuration in `lib.rs`.
2. **`StateFlags`: SIZE | POSITION | MAXIMIZED — nothing else.** Excluding `VISIBLE` is the critical one: with it set, `restore_state` calls `show()` + `set_focus()` at window-ready, before React's first paint — regressing the anti-white-flash launch flow. Excluded, the plugin never touches visibility and `app_ready` stays the only reveal path. `DECORATIONS` is static config (overlay title bar), and `FULLSCREEN` restore would trigger a macOS space transition on a hidden window at launch — quitting while fullscreen instead restores a monitor-sized normal window, which is acceptable.
3. **Main window only, via `with_filter(|label| label == "main")`.** Secondary windows (impl 0018) get unique `window-<ulid>` labels: a restore could never match a fresh ulid anyway, but unfiltered, every secondary ever opened would append a dead entry to the state file forever. The filter also keeps them out of the save cache entirely, and preserves the deliberate cascade positioning in `open_window` (`commands/window.rs:63`). The issue blesses main-only as v1.
4. **Dev/prod split via `with_filename`.** Debug builds use `.window-state-dev.json`, mirroring the `-dev` data-dir precedent (`lib.rs:114`) — `tauri dev` must not trample the packaged install's window placement. The plugin hardcodes `app_config_dir` (the same directory as prod app data on macOS), so the filename suffix is the only available split.
5. **No frontend changes.** Geometry restore is invisible to React; `app_ready`, the focus map, and multi-window registration are untouched.

## Goals

- Quit (⌘Q or dock) → relaunch: main window reappears at its last position and size, including maximized state.
- Saved monitor gone / geometry off-screen → OS-default placement at saved size (no invisible window).
- Launch flow unchanged: window stays hidden until first paint, no flash, no jump.
- Secondary windows behave exactly as today (cascade, no persistence).

## Non-Goals

- Secondary-window restore (position or which chats they held) — promote to a spec via `/feature spec` if wanted later, per the issue.
- Persisting window state across crashes/force-kills — the plugin writes only on graceful exit; a crash keeps the previous graceful-exit state (see Open Questions).
- Per-monitor or per-workspace placement profiles.

## Implementation Phases

### Phase 1 — dependency + registration

- `src-tauri/Cargo.toml`: add `tauri-plugin-window-state = "2"` (resolves 2.4.1).
- `src-tauri/src/lib.rs`: register in the plugin chain (~line 112):

```rust
.plugin(
    tauri_plugin_window_state::Builder::new()
        .with_state_flags(
            StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED,
        )
        .with_filter(|label| label == "main")
        .with_filename(if cfg!(debug_assertions) {
            ".window-state-dev.json"
        } else {
            tauri_plugin_window_state::DEFAULT_FILENAME
        })
        .build(),
)
```

### Phase 2 — verify

- `cargo fmt && cargo clippy && cargo test --workspace` (no frontend change; tsc/lint untouched).
- Manual smoke (user-run):
  - Move + resize the main window → ⌘Q → relaunch → geometry restored, no white flash, window revealed only after first paint.
  - Close the main window (red button) → quit from dock → relaunch → restored (hide-on-close path).
  - Maximize (green-button zoom) → quit → relaunch → maximized restored.
  - ⌘N secondary window → still cascades from the focused window; quit → relaunch → only main restores; state file contains only a `main` entry.
  - Move window to an external monitor → quit → unplug monitor → relaunch → window appears on the built-in display at saved size.
  - Fresh install (delete the state file) → default 1440×900 as today.
  - `tauri dev` writes `.window-state-dev.json`, release writes `.window-state.json`.

## Relevant Code

- `src-tauri/src/lib.rs:105-112` — plugin registration chain; `:114-130` — the `-dev` data-dir precedent the filename split mirrors; `:341-357` — main-window hide-on-close.
- `src-tauri/src/commands/app.rs:18` — `app_ready` show-after-paint (the flow `VISIBLE` exclusion protects).
- `src-tauri/src/commands/window.rs:18-77` — secondary-window creation + cascade positioning (excluded by the filter).
- `src-tauri/tauri.conf.json:13-24` — main-window config defaults (now first-launch-only).

## Open Questions

- **Crash resilience**: the plugin writes to disk only on `RunEvent::Exit`. If it matters in practice, a one-line `app.save_window_state(flags)` in the main-window hide-on-close branch would checkpoint geometry on every close; skipped for v1 to stay minimal.

## References

- Issue #271 — feat: restore window position and size across app restarts.
- `tauri-plugin-window-state` 2.4.1 source (`~/.cargo/registry/src/*/tauri-plugin-window-state-2.4.1/src/lib.rs`) — restore/save mechanics, `MonitorExt::intersects` off-screen check, filter-before-cache behavior.
- impl [0018](0018-multi-window.md) — secondary-window lifecycle this impl deliberately leaves alone.
