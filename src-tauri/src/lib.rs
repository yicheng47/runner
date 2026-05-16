mod cli_install;
mod commands;
mod db;
mod error;
mod event_bus;
mod model;
mod panic_hook;
mod router;
mod session;
mod shell_path;

use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(target_os = "macos")]
use tauri::menu::{AboutMetadataBuilder, PredefinedMenuItem};
use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Manager, Wry};
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
    /// Live per-mission session manager (tmux-backed). Created at app
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

    let mut log_builder = LogBuilder::new()
        .targets([
            Target::new(TargetKind::LogDir {
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
            // Session reconciliation now happens AFTER the
            // runtime is constructed (below) — for tmux-runtime
            // rows we need to query the runtime's view of the
            // pane before deciding whether to mark the row
            // stopped. Live panes survive Runner restart by
            // design (`exit-empty off` in the generated tmux
            // config); the prior portable-pty-era bulk UPDATE
            // would have killed that survival path.
            // Drop the bundled `runner` CLI into $APPDATA/runner/bin/ so
            // child PTYs find it on PATH (arch §5.3 Layer 2). Best-effort:
            // a copy failure is logged and the app keeps running. Sessions
            // spawned with no CLI on PATH will simply error out when they
            // try to invoke `runner` — surfaced as a runtime stderr from
            // the agent rather than a startup hang.
            if let Err(e) = cli_install::install_runner_cli(&app_data_dir) {
                log::error!("failed to install bundled CLI: {e}");
            }

            // Resolve the user's login-shell PATH once at startup so
            // child PTYs can find tools that live outside launchd's
            // stripped default PATH (Homebrew, mise/asdf/fnm, npm-global,
            // etc.). Best-effort: a failure or timeout just leaves
            // shell_path = None and we fall back to the inherited
            // launchd PATH. See `shell_path` module docs for the
            // launchd-strips-PATH problem this fixes.
            let shell_path = shell_path::resolve_login_shell_path();

            // Construct the session runtime (Step 9 of
            // docs/impls/0004-tmux-session-runtime.md). v1 is
            // tmux-only — Windows fails at startup with a clear
            // error; the native-pty runtime is the future Windows
            // path.
            //
            // reconcile_config() rewrites the per-app tmux.conf and
            // source-files it into a leftover server when the
            // @runner_config_version stamp is stale.
            let runtime: Arc<dyn session::runtime::SessionRuntime> = {
                #[cfg(unix)]
                {
                    let rt = session::tmux_runtime::TmuxRuntime::new(&app_data_dir).map_err(
                        |e| -> Box<dyn std::error::Error> { format!("tmux runtime: {e}").into() },
                    )?;
                    let _ = rt.reconcile_config();
                    Arc::new(rt)
                }
                #[cfg(not(unix))]
                {
                    return Err("Runner requires macOS or Linux (tmux runtime); \
                                native-pty runtime is not yet shipped"
                        .into());
                }
            };

            let sessions = session::SessionManager::new(shell_path, runtime);

            // Build the AppState up front so the mission-side
            // reattach (next block) has access to the bus + router
            // registries it needs to mount. The session-side reattach
            // still runs from the local `sessions` Arc handle.
            let state = AppState {
                db: Arc::clone(&pool),
                app_data_dir,
                sessions: Arc::clone(&sessions),
                buses: event_bus::BusRegistry::new(),
                routers: router::RouterRegistry::new(),
            };

            // Mount router + bus for every `running` mission BEFORE
            // session reattach starts. Forwarder threads begin
            // emitting `mission_*` events as soon as `pipe-pane` is
            // installed; if the bus isn't mounted yet, those events
            // get fanout-dropped. The NDJSON log is unaffected (the
            // agent writes straight through the bundled CLI) — this
            // is purely about the in-memory fanout layer.
            //
            // Returns the set of mission ids whose mount FAILED;
            // session reattach uses it to fall back to the
            // stop+mark-stopped path for those missions' alive
            // panes (matches the pre-eager-mount safety property).
            let app_handle = app.handle().clone();
            let failed_mission_ids = tauri::async_runtime::block_on(
                commands::mission::reattach_all_running_missions(&state, &app_handle),
            );

            // Reattach to any live tmux panes from a prior Runner
            // process. Rows whose pane is still alive stay
            // `running` and the manager rebuilds its forwarder
            // thread + handle for them. Rows whose pane is gone
            // (or has exited) get flipped to stopped/crashed via
            // the runtime's exit code. Mission rows whose router
            // mount failed are stopped instead.
            let events_for_reattach: Arc<dyn session::manager::SessionEvents> =
                Arc::new(session::manager::TauriSessionEvents(app.handle().clone()));
            sessions.reattach_running_sessions(
                Arc::clone(&pool),
                events_for_reattach,
                &failed_mission_ids,
                &state.app_data_dir,
            );

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
            commands::mission::mission_reset,
            commands::mission::mission_pin,
            commands::mission::mission_rename,
            commands::mission::mission_list,
            commands::mission::mission_list_summary,
            commands::mission::mission_get,
            commands::mission::mission_events_replay,
            commands::mission::mission_post_human_signal,
            commands::session::session_list,
            commands::session::session_list_recent_direct,
            commands::session::session_get,
            commands::session::session_archive,
            commands::session::session_rename,
            commands::session::session_pin,
            commands::session::session_resume,
            commands::session::session_inject_stdin,
            commands::session::session_kill,
            commands::session::session_resize,
            commands::session::session_output_snapshot,
            commands::session::session_paste_image,
            commands::session::session_start_direct,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// First log line on every app start. Captures the four things
/// triage usually wants up front: version, app_data_dir, OS/arch,
/// and tmux version.
fn log_startup_banner(app: &AppHandle, app_data_dir: &Path) {
    let pkg = app.package_info();
    let tmux = std::process::Command::new("tmux")
        .arg("-V")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "(unavailable)".to_string());
    log::info!(
        "starting {} v{} on {}-{}; app_data_dir={}; {}",
        pkg.name,
        pkg.version,
        std::env::consts::OS,
        std::env::consts::ARCH,
        app_data_dir.display(),
        tmux,
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
            .items(&[&app_menu, &edit_menu, &view_menu, &window_menu, &help_menu])
            .build()
    }
    #[cfg(not(target_os = "macos"))]
    {
        MenuBuilder::new(app).items(&[&help_menu]).build()
    }
}

/// Compute the path `tauri-plugin-log` would resolve for the LogDir
/// target, BEFORE any Tauri runtime exists. Used by the pre-builder
/// panic hook as a fallback sink so a panic during plugin init still
/// lands next to the eventual `runner.log`.
///
/// Mirrors the plugin's platform conventions:
///
/// - macOS:   `$HOME/Library/Logs/<identifier>/runner.log`
/// - Linux:   `$XDG_DATA_HOME` (or `$HOME/.local/share`) `/<identifier>/logs/runner.log`
/// - Windows: `$LOCALAPPDATA/<identifier>/logs/runner.log`
///
/// Best-effort env lookups with sane fallbacks — we'd rather write
/// a panic line into `./runner.log` than lose it.
fn default_log_path() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        home.join("Library/Logs")
            .join(APP_IDENTIFIER)
            .join("runner.log")
    }
    #[cfg(target_os = "linux")]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        base.join(APP_IDENTIFIER).join("logs").join("runner.log")
    }
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        base.join(APP_IDENTIFIER).join("logs").join("runner.log")
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    PathBuf::from("runner.log")
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
