mod cli_install;
mod commands;
mod db;
mod error;
mod event_bus;
mod mcp;
mod model;
mod panic_hook;
mod repo;
mod router;
mod session;
mod shell_path;
mod windows;

use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(target_os = "macos")]
use tauri::menu::{AboutMetadataBuilder, PredefinedMenuItem};
use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_log::{Builder as LogBuilder, RotationStrategy, Target, TargetKind};

/// Bundle identifier as declared in `tauri.conf.json`. Used by:
///
/// 1. The pre-builder fallback panic-log path (so panics that fire
///    before `tauri-plugin-log`'s setup callback runs still land in
///    the same dir the plugin itself will write to once it boots).
/// 2. The `identifier_matches_tauri_conf` test, which string-asserts
///    `tauri.conf.json` against this constant — catches a silent
///    rename in either direction.
pub const APP_IDENTIFIER: &str = "com.wycstudios.runner";

pub struct AppState {
    pub db: Arc<db::DbPool>,
    /// Root of the app's per-user data tree — `$APPDATA/runner/` on real
    /// installs, a tempdir in tests. Mission commands resolve event-log paths
    /// relative to this via `runner_core::event_log::path`.
    pub app_data_dir: PathBuf,
    /// Live per-mission session manager. Created at app
    /// start, shared across all Tauri commands and the per-session
    /// forwarder threads it spawns.
    pub sessions: Arc<session::SessionManager>,
    /// Live per-mission event-bus watchers. Mounted by `mission_start` once
    /// the opening events are durable; unmounted by `mission_stop` and on
    /// any rollback path.
    pub buses: Arc<event_bus::BusRegistry>,
    /// Live per-mission signal routers. Mounted alongside the bus so the
    /// router observes the bootstrap `mission_goal` event during initial
    /// replay and pushes the launch prompt into the lead's stdin.
    pub routers: Arc<router::RouterRegistry>,
    /// MCP server lifecycle handle (impl 0013). Unix socket listener
    /// that external clients connect to via the `runner-mcp` bridge.
    pub mcp: Arc<mcp::McpHandle>,
    /// Cross-window coordination map (impl 0018). Tracks which subject
    /// (mission / direct chat) each webview window is looking at + when it
    /// was last focused, so exactly one window owns a duplicated subject's
    /// PTY. `main` is registered in `setup`.
    pub windows: Arc<windows::WindowRegistry>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Install the panic hook BEFORE the Tauri builder. The fallback
    // path mirrors the dir `tauri-plugin-log` will resolve from the
    // bundle identifier; both writes (the `log::error!` line and the
    // direct-file append from the hook) end up next to each other.
    panic_hook::install(default_log_path());

    let default_level = if cfg!(debug_assertions) {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };
    let log_levels = std::env::var("RUST_LOG")
        .ok()
        .map(|raw| parse_rust_log(&raw, default_level))
        .unwrap_or(LogLevels {
            global: default_level,
            per_target: Vec::new(),
        });

    // `Folder` (vs `LogDir`) so dev builds write to a `-dev`-suffixed
    // directory that mirrors `app_data_dir`'s dev/prod split applied
    // in the setup callback below. `LogDir` resolves via the bundle
    // identifier with no dev suffix, so without this dev + prod
    // builds both wrote to the same `runner.log` and triage couldn't
    // tell them apart.
    let mut log_builder = LogBuilder::new()
        .targets([
            Target::new(TargetKind::Folder {
                path: log_dir_for_build(),
                file_name: Some("runner".into()),
            }),
            #[cfg(debug_assertions)]
            Target::new(TargetKind::Stdout),
        ])
        .level(log_levels.global)
        .max_file_size(10 * 1024 * 1024)
        .rotation_strategy(RotationStrategy::KeepSome(3));
    for (target, level) in log_levels.applied_targets() {
        log_builder = log_builder.level_for(target, level);
    }

    tauri::Builder::default()
        .plugin(log_builder.build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // Dev builds write to a sibling `<identifier>-dev` directory so
            // local testing can't trample a packaged install's database,
            // event logs, or bundled CLI. `cfg!(debug_assertions)` is true
            // for `tauri dev` and false for release bundles, which matches
            // how Quill separates dev vs prod data.
            let app_data_dir = {
                let base = app.path().app_data_dir()?;
                if cfg!(debug_assertions) {
                    let dev_name = match base.file_name().and_then(|s| s.to_str()) {
                        Some(name) => format!("{name}-dev"),
                        None => "runner-dev".to_string(),
                    };
                    base.with_file_name(dev_name)
                } else {
                    base
                }
            };
            std::fs::create_dir_all(&app_data_dir)?;

            // First line of every app start. Triage-from-log starts here.
            log_startup_banner(app.handle(), &app_data_dir);

            let db_path = app_data_dir.join("runner.db");
            let pool = Arc::new(db::open_pool(&db_path)?);
            // Session startup cleanup happens after the runtime is
            // constructed. Under the in-process PTY runtime, child
            // processes die with the prior app process, so stale
            // `running` rows are demoted below.
            // Drop the bundled agent/MCP CLIs into $APPDATA/runner/bin/.
            // Child PTYs find `runner` on PATH (arch §5.3 Layer 2), while
            // Claude/Codex configs point at `runner-mcp`. Best-effort: a
            // copy failure is logged and the app keeps running.
            if let Err(e) = cli_install::install_runner_cli(&app_data_dir) {
                log::error!("failed to install bundled agent CLI: {e}");
            }
            if let Err(e) = cli_install::install_mcp_cli(&app_data_dir) {
                log::error!("failed to install bundled MCP CLI: {e}");
            }

            // Snapshot the user's login-shell env once at startup so
            // child PTYs see the same PATH + proxy vars that
            // Terminal.app's children would. Covers both the
            // GUI-launch shim-discovery problem (Homebrew /
            // mise / asdf / fnm / npm-global on a launchd-stripped
            // PATH) and the claude/codex-login-behind-VPN problem
            // (HTTPS_PROXY / NO_PROXY etc. set in rc files).
            // Best-effort: a failure or timeout just leaves the
            // snapshot empty and we fall back to launchd's env.
            let login_shell_env = shell_path::resolve_login_shell_env();

            // Construct the in-process PTY runtime
            // (docs/impls/archive/0011). v1 is unix-only — Windows fails at
            // startup with a clear error.
            let runtime: Arc<dyn session::runtime::SessionRuntime> = {
                #[cfg(unix)]
                {
                    log::info!("session runtime: pty (in-process, impl 0011)");
                    Arc::new(session::pty_runtime::PtyRuntime::new())
                }
                #[cfg(not(unix))]
                {
                    return Err("Runner requires macOS or Linux; \
                                Windows support is pending impl 0011's \
                                cross-platform pass"
                        .into());
                }
            };

            let sessions = session::SessionManager::new(login_shell_env, runtime);

            // Build the AppState up front so the startup mission-bus
            // remount has access to the bus + router registries it
            // needs.
            let buses = event_bus::BusRegistry::new();
            let routers = router::RouterRegistry::new();
            let mcp_handle = Arc::new(mcp::McpHandle::new());
            // Window registry seeded with `main` — it exists before any
            // frontend reports a subject, and the snapshot must reflect it
            // from the first broadcast. Shared with `McpState` so the
            // MCP-reconstructed `AppState` sees the same map.
            let window_registry = Arc::new(windows::WindowRegistry::new());
            window_registry.register("main");
            let mcp_state = mcp::state::McpState {
                db: Arc::clone(&pool),
                app_data_dir: app_data_dir.clone(),
                sessions: Arc::clone(&sessions),
                buses: Arc::clone(&buses),
                routers: Arc::clone(&routers),
                mcp: Arc::clone(&mcp_handle),
                windows: Arc::clone(&window_registry),
                app_handle: app.handle().clone(),
            };
            if let Err(e) = mcp_handle.start(&app_data_dir.join("mcp.sock"), mcp_state) {
                log::error!("mcp: failed to start listener: {e}");
            }

            let state = AppState {
                db: Arc::clone(&pool),
                app_data_dir,
                sessions: Arc::clone(&sessions),
                buses,
                routers,
                mcp: mcp_handle,
                windows: window_registry,
            };

            // Mount router + bus for every `running` mission before
            // stale session rows are demoted. The NDJSON log is the
            // durable source of truth; this is purely about restoring
            // the in-memory fanout layer for mission events.
            let app_handle = app.handle().clone();
            tauri::async_runtime::block_on(commands::mission::mount_all_running_mission_routers(
                &state,
                &app_handle,
            ));

            // Agents die with this Tauri process under the pty
            // runtime — any `running` row in the DB at this point
            // is from a prior process. Demote them to `stopped`
            // so the sidebar surfaces them with a Resume
            // affordance (impl 0011 §"Tauri startup").
            #[cfg(unix)]
            {
                if let Err(e) = session::pty_runtime::cleanup_stale_running_rows_on_startup(&pool) {
                    log::warn!("pty runtime startup cleanup failed: {e}");
                }
            }

            app.manage(state);

            // Build the app menu and wire the `runner_logs_reveal`
            // menu item to the same handler the Settings →
            // Diagnostics button calls. Done in `setup` (not at the
            // builder level) so we get a real `AppHandle` for the
            // menu's child item builders.
            let menu = build_menu(app.handle())?;
            app.set_menu(menu)?;
            app.on_menu_event(|app, ev| {
                if ev.id() == "runner_logs_reveal" {
                    if let Err(e) = commands::app::reveal_logs_dir(app) {
                        log::error!("reveal logs failed: {e}");
                    }
                } else if ev.id() == "window_new" {
                    let state = app.state::<AppState>();
                    if let Err(e) = commands::window::open_window(app, state.inner(), None, None) {
                        log::error!("new window failed: {e}");
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::app_ready,
            commands::app::runner_logs_reveal,
            commands::crew::crew_list,
            commands::crew::crew_get,
            commands::crew::crew_create,
            commands::crew::crew_update,
            commands::crew::crew_delete,
            commands::runner::runner_list,
            commands::runner::runner_list_with_activity,
            commands::runner::runner_get,
            commands::runner::runner_get_by_handle,
            commands::runner::runner_create,
            commands::runner::runner_update,
            commands::runner::runner_delete,
            commands::runner::runner_activity,
            commands::runtime::runtime_list,
            commands::slot::slot_list,
            commands::slot::runner_crews_list,
            commands::slot::slot_create,
            commands::slot::slot_update,
            commands::slot::slot_delete,
            commands::slot::slot_set_lead,
            commands::slot::slot_reorder,
            commands::mission::mission_start,
            commands::mission::mission_attach,
            commands::mission::mission_stop,
            commands::mission::mission_archive,
            commands::mission::mission_unarchive,
            commands::mission::mission_reset,
            commands::mission::mission_pin,
            commands::mission::mission_rename,
            commands::mission::mission_list,
            commands::mission::mission_list_archived,
            commands::mission::mission_list_summary,
            commands::mission::mission_get,
            commands::mission::mission_events_replay,
            commands::mission::mission_post_human_signal,
            commands::session::session_list,
            commands::session::session_list_recent_direct,
            commands::session::session_list_archived,
            commands::session::session_get,
            commands::session::session_archive,
            commands::session::session_unarchive,
            commands::session::session_rename,
            commands::session::session_pin,
            commands::session::session_resume,
            commands::session::session_inject_stdin,
            commands::session::session_kill,
            commands::session::session_resize,
            commands::session::session_output_snapshot,
            commands::session::session_replay_watermark,
            commands::session::session_paste_image,
            commands::session::session_start_direct,
            commands::session::session_start_runtime,
            commands::mcp::mcp_integration_status,
            commands::mcp::mcp_set_integration,
            commands::mcp::mcp_config_snippet,
            commands::window::window_open,
            commands::window::window_focus_other,
            commands::window::window_report_subjects,
            commands::window::window_list_subjects,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // On graceful quit, stop any `running` direct-chat
            // sessions before the runtime drops. Under the pty
            // runtime, this fires SIGTERM-via-ChildKiller so
            // children get a chance to flush conversation state
            // (claude-code session file, etc.) before the
            // master-fd-close SIGHUP cascade lands. We tolerate kill
            // failures: best-effort path, and the startup cleanup is
            // the safety net on next launch.
            match event {
                tauri::RunEvent::WindowEvent {
                    label,
                    event: tauri::WindowEvent::CloseRequested { api, .. },
                    ..
                } if label == "main" => {
                    api.prevent_close();
                    hide_main_window_on_close(app_handle);
                    // main hides rather than destroys, so it never emits
                    // `Destroyed` to unregister. Demote it (impl 0018) so a
                    // visible duplicate window becomes primary and mounts its
                    // PTY; main keeps its subject, so reopening + focusing it
                    // reclaims ownership via the `Focused(true)` hook.
                    if let Some(state) = app_handle.try_state::<AppState>() {
                        state.windows.mark_hidden(&label);
                    }
                    broadcast_focus_map(app_handle);
                }
                // Any window gaining focus becomes primary for whatever
                // subject it's on (spec decision 2). Secondary windows close
                // for real (no main-style prevent), and the `Destroyed`
                // unregister below promotes the next-most-recent survivor.
                tauri::RunEvent::WindowEvent {
                    label,
                    event: tauri::WindowEvent::Focused(true),
                    ..
                } => {
                    if let Some(state) = app_handle.try_state::<AppState>() {
                        state.windows.mark_focused(&label);
                    }
                    broadcast_focus_map(app_handle);
                }
                tauri::RunEvent::WindowEvent {
                    label,
                    event: tauri::WindowEvent::Destroyed,
                    ..
                } => {
                    if let Some(state) = app_handle.try_state::<AppState>() {
                        state.windows.unregister(&label);
                    }
                    broadcast_focus_map(app_handle);
                }
                tauri::RunEvent::ExitRequested { .. } => {
                    stop_running_sessions_on_quit(app_handle);
                    if let Some(state) = app_handle.try_state::<AppState>() {
                        state.mcp.stop();
                    }
                }
                #[cfg(target_os = "macos")]
                tauri::RunEvent::Reopen {
                    has_visible_windows,
                    ..
                } => {
                    if !has_visible_windows {
                        if let Err(e) = commands::app::show_main_window(app_handle) {
                            log::error!("reopen main window failed: {e}");
                        }
                    }
                }
                tauri::RunEvent::Resumed => {
                    let _ = app_handle.emit("app/resumed", ());
                }
                _ => {}
            }
        });
}

/// Broadcast the current window→subject map to every webview. Called after
/// every registry mutation (lifecycle hooks + window commands) so all windows
/// converge on a consistent picture of who owns what. Broadcast, not
/// targeted: each window filters by its own subject (spec decision 5).
pub(crate) fn broadcast_focus_map(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let snapshot = state.windows.snapshot();
    if let Err(e) = app.emit("window_focus_map", snapshot) {
        log::error!("broadcast window_focus_map failed: {e}");
    }
}

fn hide_main_window_on_close(app_handle: &AppHandle) {
    let Some(window) = app_handle.get_webview_window("main") else {
        log::warn!("window close requested, but main window was not found");
        return;
    };
    if let Err(e) = window.hide() {
        log::error!("hide main window failed: {e}");
    }
}

/// On-quit hook body. Walks the DB for `running` direct-chat
/// sessions and asks `SessionManager` to kill each one. Mission-
/// scoped rows are left alone — their lifecycle is owned by
/// `mission_stop` flows and v1 keeps router/event-bus migration
/// out of scope (impl 0011 §"Mission sessions").
fn stop_running_sessions_on_quit(app_handle: &AppHandle) {
    let state = match app_handle.try_state::<AppState>() {
        Some(s) => s,
        None => return,
    };
    let ids: Vec<String> = match state.db.get().ok().and_then(|conn| {
        conn.prepare(
            "SELECT id FROM sessions
                WHERE status = 'running' AND mission_id IS NULL",
        )
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([], |r| r.get::<_, String>(0))
                .ok()
                .map(|rows| rows.filter_map(Result::ok).collect())
        })
    }) {
        Some(v) => v,
        None => return,
    };
    if ids.is_empty() {
        return;
    }
    log::info!(
        "on-quit: stopping {} running direct-chat session(s)",
        ids.len()
    );
    for id in &ids {
        if let Err(e) = state.sessions.kill(id) {
            log::warn!("on-quit kill {id}: {e}");
        }
    }
}

/// First log line on every app start. Captures the things triage
/// usually wants up front: version, app_data_dir, OS/arch.
fn log_startup_banner(app: &AppHandle, app_data_dir: &Path) {
    let pkg = app.package_info();
    log::info!(
        "starting {} v{} on {}-{}; app_data_dir={}",
        pkg.name,
        pkg.version,
        std::env::consts::OS,
        std::env::consts::ARCH,
        app_data_dir.display(),
    );
}

/// Build the application menu. On macOS we recreate the full
/// standard menu (App / Edit / View / Window / Help) so the system
/// shortcuts (Cmd+C/V/X, Cmd+W, Cmd+Q, fullscreen, …) stay wired up
/// — calling `set_menu` with a smaller menu would strip them. On
/// other platforms we only attach Help; standard shortcuts there
/// flow through the webview without a menu bar.
fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let reveal_logs =
        MenuItemBuilder::with_id("runner_logs_reveal", "Reveal logs in Finder").build(app)?;
    let help_menu = SubmenuBuilder::new(app, "Help")
        .item(&reveal_logs)
        .build()?;

    // File → New Window owns Cmd+N at the OS/menu level (impl 0018). The
    // accelerator is handled by the menu, not a JS keydown handler, so the
    // shortcut works regardless of webview focus and can't double-fire.
    let new_window = MenuItemBuilder::with_id("window_new", "New Window")
        .accelerator("CmdOrCtrl+N")
        .build(app)?;
    let file_menu = SubmenuBuilder::new(app, "File").item(&new_window).build()?;

    #[cfg(target_os = "macos")]
    {
        let pkg = app.package_info();
        let about_meta = AboutMetadataBuilder::new()
            .name(Some(pkg.name.clone()))
            .version(Some(pkg.version.to_string()))
            .build();
        let about = PredefinedMenuItem::about(app, Some("About Runner"), Some(about_meta))?;

        let app_menu = SubmenuBuilder::new(app, "Runner")
            .item(&about)
            .separator()
            .services()
            .separator()
            .hide()
            .hide_others()
            .show_all()
            .separator()
            .quit()
            .build()?;

        let edit_menu = SubmenuBuilder::new(app, "Edit")
            .undo()
            .redo()
            .separator()
            .cut()
            .copy()
            .paste()
            .select_all()
            .build()?;

        let view_menu = SubmenuBuilder::new(app, "View").fullscreen().build()?;

        let window_menu = SubmenuBuilder::new(app, "Window")
            .minimize()
            .maximize()
            .separator()
            .close_window()
            .build()?;

        MenuBuilder::new(app)
            .items(&[
                &app_menu,
                &file_menu,
                &edit_menu,
                &view_menu,
                &window_menu,
                &help_menu,
            ])
            .build()
    }
    #[cfg(not(target_os = "macos"))]
    {
        MenuBuilder::new(app)
            .items(&[&file_menu, &help_menu])
            .build()
    }
}

/// The bundle-id segment for log + app-data paths, with a `-dev`
/// suffix in debug builds. Mirrors the dev/prod split that
/// `app_data_dir` gets in the Tauri setup callback so a `tauri dev`
/// session can't trample a packaged install's `runner.log` (or, the
/// other way, contaminate prod-build triage with stale dev lines).
/// Used by both the panic-hook fallback path and the plugin-log
/// builder.
fn bundle_segment() -> String {
    if cfg!(debug_assertions) {
        format!("{APP_IDENTIFIER}-dev")
    } else {
        APP_IDENTIFIER.to_string()
    }
}

/// Directory the plugin-log `Folder` target writes to. Computed
/// before the Tauri builder exists (the plugin needs the path at
/// build time, not at setup time), so this can't go through
/// `app.path().app_log_dir()` and has to mirror the platform
/// conventions explicitly.
fn log_dir_for_build() -> PathBuf {
    let segment = bundle_segment();
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        home.join("Library/Logs").join(segment)
    }
    #[cfg(target_os = "linux")]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        base.join(segment).join("logs")
    }
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        base.join(segment).join("logs")
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    PathBuf::from(".")
}

/// Compute the full path of the file `tauri-plugin-log` writes to.
/// Used by the pre-builder panic hook as a fallback sink so a panic
/// during plugin init still lands next to the eventual `runner.log`.
fn default_log_path() -> PathBuf {
    log_dir_for_build().join("runner.log")
}

/// Parsed `RUST_LOG` directives. `global` always carries a value
/// (the caller's default if the env var is unset / unparseable);
/// `per_target` is empty unless one or more `target=level` pairs
/// were present.
struct LogLevels {
    global: log::LevelFilter,
    per_target: Vec<(String, log::LevelFilter)>,
}

impl LogLevels {
    /// Expand parsed pairs into the final list handed to
    /// `Builder::level_for`. Aliases `runner` → also `runner_lib`, the
    /// real crate name (set by `[lib] name` in `src-tauri/Cargo.toml`).
    ///
    /// Spec §Phase 2 documents `RUST_LOG=runner=debug` as the dev
    /// escape hatch, so we honor that exact form. Without the alias
    /// the directive would silently no-op against
    /// `runner_lib::*` targets, which is where every `log::` macro in
    /// this crate actually emits from.
    ///
    /// A user-supplied `runner_lib=…` directive is preserved as-is —
    /// `level_for` is idempotent per target, so a duplicate from
    /// `runner` aliasing on top would only re-assert the same level.
    fn applied_targets(&self) -> Vec<(String, log::LevelFilter)> {
        let mut out = Vec::with_capacity(self.per_target.len());
        for (target, level) in &self.per_target {
            out.push((target.clone(), *level));
            if target == "runner" {
                out.push(("runner_lib".to_string(), *level));
            }
        }
        out
    }
}

/// Tiny `RUST_LOG` parser. Supports the two forms the spec calls
/// out:
///
/// ```text
/// RUST_LOG=debug                 → global = Debug
/// RUST_LOG=runner=debug,info     → per-target runner=Debug, global = Info
/// ```
///
/// More-elaborate `env_logger` grammar (regex filters, span scopes,
/// etc.) is intentionally out of scope. Unrecognized fragments are
/// silently skipped — they fall back to the caller-supplied default
/// instead of taking the whole filter down with them.
fn parse_rust_log(input: &str, default_global: log::LevelFilter) -> LogLevels {
    let mut global = default_global;
    let mut per_target = Vec::new();
    for part in input.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.split_once('=') {
            Some((target, level)) => {
                if let Some(lf) = parse_level(level) {
                    per_target.push((target.trim().to_string(), lf));
                }
            }
            None => {
                if let Some(lf) = parse_level(part) {
                    global = lf;
                }
            }
        }
    }
    LogLevels { global, per_target }
}

fn parse_level(s: &str) -> Option<log::LevelFilter> {
    match s.trim().to_ascii_lowercase().as_str() {
        "off" => Some(log::LevelFilter::Off),
        "error" => Some(log::LevelFilter::Error),
        "warn" => Some(log::LevelFilter::Warn),
        "info" => Some(log::LevelFilter::Info),
        "debug" => Some(log::LevelFilter::Debug),
        "trace" => Some(log::LevelFilter::Trace),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Identifier ↔ log-path safety net: `tauri-plugin-log` resolves
    // `LogDir` via the bundle identifier in tauri.conf.json, and our
    // pre-builder fallback log path uses the same constant. A rename
    // that touches only one side would silently send logs to a
    // different dir than the panic-hook fallback — this test wedges
    // both sides against `APP_IDENTIFIER`.
    #[test]
    fn identifier_matches_tauri_conf() {
        let raw = include_str!("../tauri.conf.json");
        let v: serde_json::Value = serde_json::from_str(raw).expect("parse tauri.conf.json");
        let ident = v
            .get("identifier")
            .and_then(|s| s.as_str())
            .expect("identifier field");
        assert_eq!(ident, APP_IDENTIFIER);
    }

    #[test]
    fn bundle_segment_appends_dev_suffix_in_debug_builds() {
        // Wedges in the dev/prod log-dir split so a future refactor that
        // drops the suffix doesn't silently start writing dev runs into
        // the packaged install's `runner.log`. `cfg!(debug_assertions)`
        // is true for `cargo test` so the assertion checks the debug
        // arm directly.
        let seg = bundle_segment();
        assert!(
            seg.ends_with("-dev"),
            "expected -dev suffix in debug build, got {seg:?}",
        );
        assert!(
            seg.starts_with(APP_IDENTIFIER),
            "segment must start with bundle id, got {seg:?}",
        );
    }

    #[test]
    fn log_dir_for_build_lives_under_bundle_segment() {
        // Sanity-check that `log_dir_for_build` actually composes the
        // dev-aware segment — easy to break if someone refactors the
        // path computation without rerouting through `bundle_segment`.
        let dir = log_dir_for_build();
        let seg = bundle_segment();
        assert!(
            dir.to_string_lossy().contains(&seg),
            "log dir {dir:?} must contain bundle segment {seg:?}",
        );
    }

    #[test]
    fn parse_rust_log_global_only() {
        let l = parse_rust_log("debug", log::LevelFilter::Info);
        assert_eq!(l.global, log::LevelFilter::Debug);
        assert!(l.per_target.is_empty());
    }

    #[test]
    fn parse_rust_log_per_target_with_default() {
        let l = parse_rust_log("runner=debug,info", log::LevelFilter::Warn);
        assert_eq!(l.global, log::LevelFilter::Info);
        assert_eq!(
            l.per_target,
            vec![("runner".to_string(), log::LevelFilter::Debug)]
        );
    }

    #[test]
    fn parse_rust_log_invalid_falls_back_to_default() {
        let l = parse_rust_log("garbage,also-garbage=nope", log::LevelFilter::Info);
        assert_eq!(l.global, log::LevelFilter::Info);
        assert!(l.per_target.is_empty());
    }

    // Spec phase-2 escape hatch: `RUST_LOG=runner=debug` must actually
    // bind to the `runner_lib::*` targets every `log::` macro in this
    // crate emits from. The `[lib] name = "runner"` in Cargo.toml is
    // "runner" on the dependency-graph side but the resulting crate
    // module path is `runner_lib`. Alias both so the documented
    // directive works without the user having to know the lib rename.
    #[test]
    fn runner_alias_expands_to_runner_lib() {
        let l = parse_rust_log("runner=debug", log::LevelFilter::Info);
        let applied = l.applied_targets();
        assert!(
            applied.contains(&("runner".to_string(), log::LevelFilter::Debug)),
            "applied set must contain runner; got {applied:?}",
        );
        assert!(
            applied.contains(&("runner_lib".to_string(), log::LevelFilter::Debug)),
            "applied set must contain runner_lib alias; got {applied:?}",
        );
    }

    #[test]
    fn runner_lib_directive_does_not_double_apply() {
        // If a user writes `runner_lib=debug` directly, we don't add a
        // synthetic `runner` entry — only the alias goes the other
        // direction. (`level_for` is idempotent per target anyway, but
        // keeping the applied set tight makes the test cheap to read.)
        let l = parse_rust_log("runner_lib=debug", log::LevelFilter::Info);
        let applied = l.applied_targets();
        assert_eq!(
            applied,
            vec![("runner_lib".to_string(), log::LevelFilter::Debug)]
        );
    }
}
