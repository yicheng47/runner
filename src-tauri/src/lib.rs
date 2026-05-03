mod cli_install;
mod commands;
mod db;
mod error;
mod event_bus;
mod model;
mod router;
mod session;

use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager;

pub struct AppState {
    pub db: Arc<db::DbPool>,
    /// Root of the app's per-user data tree — `$APPDATA/runner/` on real
    /// installs, a tempdir in tests. Mission commands resolve event-log paths
    /// relative to this via `runner_core::event_log::path`.
    pub app_data_dir: PathBuf,
    /// Live per-mission PTY sessions. Created at app start, shared across
    /// all Tauri commands and the reader threads they spawn.
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
            // Reconcile orphaned sessions: any row still marked `running`
            // is from a previous process whose SessionManager died with it,
            // so the child PTY is gone too. Mark them stopped so the
            // sidebar's `direct_session_id` query and the chat page agree
            // with reality. Without this, post-restart clicks land on a
            // session id the live SessionManager doesn't know about and
            // every action returns "session not found".
            {
                let conn = pool.get()?;
                conn.execute(
                    "UPDATE sessions
                        SET status = 'stopped',
                            stopped_at = COALESCE(stopped_at, ?1)
                      WHERE status = 'running'",
                    rusqlite::params![chrono::Utc::now().to_rfc3339()],
                )?;
            }
            // Drop the bundled `runner` CLI into $APPDATA/runner/bin/ so
            // child PTYs find it on PATH (arch §5.3 Layer 2). Best-effort:
            // a copy failure is logged and the app keeps running. Sessions
            // spawned with no CLI on PATH will simply error out when they
            // try to invoke `runner` — surfaced as a runtime stderr from
            // the agent rather than a startup hang.
            if let Err(e) = cli_install::install_runner_cli(&app_data_dir) {
                eprintln!("runner: failed to install bundled CLI: {e}");
            }

            app.manage(AppState {
                db: pool,
                app_data_dir,
                sessions: session::SessionManager::new(),
                buses: event_bus::BusRegistry::new(),
                routers: router::RouterRegistry::new(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
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
            commands::session::session_archive,
            commands::session::session_rename,
            commands::session::session_pin,
            commands::session::session_resume,
            commands::session::session_inject_stdin,
            commands::session::session_kill,
            commands::session::session_resize,
            commands::session::session_output_snapshot,
            commands::session::session_start_direct,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
