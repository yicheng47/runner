mod cli_install;
mod commands;
mod db;
mod error;
mod event_bus;
mod model;
mod router;
mod session;
mod shell_path;

use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager;

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
    tauri::Builder::default()
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
                eprintln!("runner: failed to install bundled CLI: {e}");
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
            );

            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::app_ready,
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
