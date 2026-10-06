use anyhow::{Context as _, Result};
pub use runner_core::daemon_process::NativePaths;
use runner_core::protocol::DaemonClient;
use std::path::PathBuf;

pub fn native_paths() -> Result<NativePaths> {
    Ok(NativePaths::resolve()?)
}

#[derive(Clone)]
pub struct ClientHost {
    pub client: DaemonClient,
    pub app_data_dir: PathBuf,
}

pub fn connect(
    paths: &NativePaths,
) -> Result<(
    ClientHost,
    std::sync::Arc<runner_core::protocol::managed::ManagedTransport>,
)> {
    let source =
        runner_core::cli_install::locate_source(runner_core::cli_install::AGENT_SOURCE_BIN_NAME)?
            .context("bundled CLI sidecar unavailable; build runner-cli first")?;
    let hash_source = source.clone();
    let hash =
        std::thread::spawn(move || runner_core::daemon_process::executable_hash(&hash_source))
            .join()
            .map_err(|_| anyhow::anyhow!("sidecar hash worker panicked"))??;
    let connection = runner_core::protocol::managed::ManagedTransport::connect(
        runner_core::daemon_process::Launch::new(paths.clone(), source, true),
        hash,
    )?;
    Ok((
        ClientHost {
            client: connection.client(),
            app_data_dir: paths.app_data_dir.clone(),
        },
        connection,
    ))
}
#[cfg(target_os = "macos")]
pub fn install_wake(client: DaemonClient) {
    crate::wake::observe_wake(move || {
        let _ = client.app_woke();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use runner_daemon::daemon::boot::{boot_core, stop_running_sessions_on_quit};
    use runner_daemon::daemon::resume::consume_launch_claims;
    use runner_daemon::{db, repo, session, shell_path};
    use std::path::Path;
    use std::sync::{Arc, RwLock};
    fn paths_for_home(home: &Path, debug: bool) -> NativePaths {
        NativePaths::for_home(home, debug)
    }

    use std::cell::{Cell, RefCell};
    use std::collections::HashSet;

    fn launch_claim(session_id: &str, shell: bool) -> repo::session::ResumeOnLaunchClaim {
        repo::session::ResumeOnLaunchClaim {
            session_id: session_id.to_owned(),
            shell,
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn paths_match_tauri_bundle_convention() {
        let release = paths_for_home(Path::new("/Users/tester"), false);
        assert_eq!(
            release.app_data_dir,
            Path::new("/Users/tester/Library/Application Support/com.wycstudios.runner")
        );
        assert_eq!(
            release.log_dir,
            Path::new("/Users/tester/Library/Logs/com.wycstudios.runner")
        );

        let debug = paths_for_home(Path::new("/Users/tester"), true);
        assert!(debug.app_data_dir.ends_with("com.wycstudios.runner-dev"));
        assert!(debug.log_dir.ends_with("com.wycstudios.runner-dev"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn paths_use_xdg_data_home_on_linux() {
        let home = Path::new("/home/tester");
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"));
        for debug in [false, true] {
            let paths = paths_for_home(home, debug);
            let segment = if debug {
                "com.wycstudios.runner-dev"
            } else {
                "com.wycstudios.runner"
            };
            assert_eq!(paths.app_data_dir, base.join(segment));
            assert_eq!(paths.log_dir, paths.app_data_dir.join("logs"));
        }
    }

    #[cfg(windows)]
    #[test]
    fn paths_use_roaming_app_data_on_windows() {
        let home = Path::new(r"C:\Users\tester");
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"));
        for debug in [false, true] {
            let paths = paths_for_home(home, debug);
            let segment = if debug {
                "com.wycstudios.runner-dev"
            } else {
                "com.wycstudios.runner"
            };
            assert_eq!(paths.app_data_dir, base.join(segment));
            assert_eq!(paths.log_dir, paths.app_data_dir.join("logs"));
        }
    }

    #[test]
    fn startup_removes_stale_mcp_bridge_and_tolerates_a_missing_one() {
        let temp = tempfile::tempdir().unwrap();
        let first = NativePaths::new(temp.path().join("first"), temp.path().join("first-logs"));
        let bridge = first
            .app_data_dir
            .join("bin")
            .join(runner_daemon::cli_install::MCP_DEST_BIN_NAME);
        std::fs::create_dir_all(bridge.parent().unwrap()).unwrap();
        std::fs::write(&bridge, "stale bridge").unwrap();
        let _core = boot_core(&first, Vec::new()).unwrap();
        assert!(!bridge.exists());

        let second = NativePaths::new(temp.path().join("second"), temp.path().join("second-logs"));
        let _core = boot_core(&second, Vec::new()).unwrap();
        assert!(!second
            .app_data_dir
            .join("bin")
            .join(runner_daemon::cli_install::MCP_DEST_BIN_NAME)
            .exists());
    }

    #[test]
    fn quit_preparation_stamps_running_sessions_and_requeues_claims() {
        let temp = tempfile::tempdir().unwrap();
        let pool = Arc::new(db::open_pool(&temp.path().join("runner.db")).unwrap());
        {
            let conn = pool.get().unwrap();
            let timestamp = "2026-08-18T00:00:00Z".parse().unwrap();
            runner_daemon::repo::role::insert(
                &conn,
                &runner_daemon::repo::role::RoleRow {
                    id: "r1".into(),
                    handle: "alpha".into(),
                    display_name: "Alpha".into(),
                    runtime: "shell".into(),
                    command: "/bin/cat".into(),
                    args_json: Some(Vec::new()),
                    working_dir: None,
                    system_prompt: None,
                    env_json: Some(Default::default()),
                    model: None,
                    effort: None,
                    codex_speed: None,
                    created_at: timestamp,
                    updated_at: timestamp,
                },
            )
            .unwrap();
            let mut running = runner_daemon::repo::session::SessionRowDb::new_running("s1".into());
            running.role_id = Some("r1".into());
            running.started_at = Some(timestamp);
            runner_daemon::repo::session::insert(&conn, &running).unwrap();
            let mut claimed =
                runner_daemon::repo::session::SessionRowDb::new_running("claimed".into());
            claimed.role_id = Some("r1".into());
            claimed.status = runner_daemon::model::SessionStatus::Stopped;
            claimed.started_at = Some("2026-08-18T00:00:01Z".parse().unwrap());
            claimed.resume_on_launch = true;
            runner_daemon::repo::session::insert(&conn, &claimed).unwrap();
            runner_daemon::repo::session::mark_resume_on_launch_claimed(&conn, "claimed").unwrap();
        }
        let runtime: Arc<dyn session::runtime::SessionRuntime> =
            Arc::new(session::pty_runtime::PtyRuntime::new());
        let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
        let runtime_discovery =
            Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
        let core = crate::test_support::core(
            Arc::clone(&pool),
            PathBuf::new(),
            session::SessionManager::new(
                Arc::clone(&runtime_shell_env),
                Arc::clone(&runtime_discovery),
                runtime,
            ),
            runtime_shell_env,
            runtime_discovery,
        );

        stop_running_sessions_on_quit(&core).unwrap();

        let conn = pool.get().unwrap();
        for id in ["s1", "claimed"] {
            let stamp: i64 = conn
                .query_row(
                    "SELECT resume_on_launch FROM sessions WHERE id = ?1",
                    [id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(stamp, 1, "{id} must be pending for the next launch");
        }
    }

    #[test]
    fn launch_claim_consumer_clears_gated_chats_but_drains_shells_when_disabled() {
        let cleared = Cell::new(false);
        let claims = RefCell::new(vec![None, Some(launch_claim("shell-session", true))]);
        let resumed = RefCell::new(Vec::new());
        let report = consume_launch_claims(
            false,
            || {
                cleared.set(true);
                Ok(())
            },
            || Ok(claims.borrow_mut().pop().unwrap()),
            |session_id| {
                resumed.borrow_mut().push(session_id.to_owned());
                Ok(())
            },
            || {},
        )
        .unwrap();

        assert_eq!(report.resumed, ["shell-session"]);
        assert!(report.errors.is_empty());
        assert!(cleared.get());
        assert_eq!(&*resumed.borrow(), &["shell-session"]);
    }

    #[test]
    fn launch_claim_consumer_defers_drawer_shell_but_drains_pane_shell() {
        let temp = tempfile::tempdir().unwrap();
        let pool = db::open_pool(&temp.path().join("runner.db")).unwrap();
        {
            let conn = pool.get().unwrap();
            conn.execute(
                "INSERT INTO sessions
                    (id, status, started_at, agent_runtime, agent_command, resume_on_launch)
                 VALUES
                    ('drawer-shell', 'stopped', '2026-09-04T00:00:00Z',
                     'shell', '/bin/zsh', 1),
                    ('pane-shell', 'stopped', '2026-09-04T00:00:00Z',
                     'shell', '/bin/zsh', 1)",
                [],
            )
            .unwrap();
            repo::node::create_tab(
                &conn,
                None,
                "terminal",
                0,
                r#"{"preset":"single","slots":["pane-shell"],"sizes":{},"drawer":{"open":false,"height":280,"shells":["drawer-shell"],"active":0}}"#,
            )
            .unwrap();
        }
        let drawer_session_ids = {
            let conn = pool.get().unwrap();
            repo::node::list(&conn)
                .unwrap()
                .iter()
                .flat_map(repo::node::drawer_session_ids)
                .collect::<HashSet<_>>()
        };

        let report = consume_launch_claims(
            true,
            || panic!("enabled launch must not clear claims"),
            || {
                let mut conn = pool.get().unwrap();
                Ok(repo::session::take_resume_on_launch_excluding(
                    &mut conn,
                    &drawer_session_ids,
                )?)
            },
            |session_id| {
                let conn = pool.get().unwrap();
                repo::session::finish_resume_on_launch(&conn, session_id)
                    .map(drop)
                    .map_err(|error| error.to_string())
            },
            || {},
        )
        .unwrap();

        assert_eq!(report.resumed, ["pane-shell"]);
        let conn = pool.get().unwrap();
        let stamps = conn
            .prepare("SELECT id, resume_on_launch FROM sessions ORDER BY id")
            .unwrap()
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            stamps,
            [("drawer-shell".into(), 1), ("pane-shell".into(), 0)]
        );
    }

    #[test]
    fn launch_claim_consumer_skips_shell_stagger_and_continues_after_failure() {
        let claims = RefCell::new(vec![
            None,
            Some(launch_claim("session-b", false)),
            Some(launch_claim("shell-session", true)),
            Some(launch_claim("session-a", false)),
        ]);
        let attempts = RefCell::new(Vec::new());
        let trace = RefCell::new(Vec::new());
        let report = consume_launch_claims(
            true,
            || panic!("enabled launch must not clear claims"),
            || Ok(claims.borrow_mut().pop().unwrap()),
            |session_id| {
                attempts.borrow_mut().push(session_id.to_owned());
                trace.borrow_mut().push(format!("resume:{session_id}"));
                if session_id == "session-a" {
                    Err("rejected key".into())
                } else {
                    Ok(())
                }
            },
            || trace.borrow_mut().push("wait".into()),
        )
        .unwrap();

        assert_eq!(
            &*attempts.borrow(),
            &["session-a", "shell-session", "session-b"]
        );
        assert_eq!(
            &*trace.borrow(),
            &[
                "resume:session-a",
                "resume:shell-session",
                "wait",
                "resume:session-b"
            ]
        );
        assert_eq!(report.resumed, ["shell-session", "session-b"]);
        assert_eq!(report.errors, ["session-a: rejected key"]);
    }
}
