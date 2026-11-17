use crate::files::fixture::git;
use crate::model::SessionStatus;
use crate::ops::{mission, project, session};
use crate::repo;
use crate::session::runtime::{
    OutputStream, RuntimeError, RuntimeResult, RuntimeSession, SessionRuntime,
    SessionStatus as PtyStatus, SpawnSpec,
};
use crate::session::SessionManager;
use crate::test_support::{insert_test_role, insert_test_slot, row_count, test_core_in};
use crate::AppCore;
use runner_core::app_paths::{normalize_checkout_path, same_normalized_checkout_path};
use runner_core::protocol::mission::{MissionAddRoleInput, MissionStart, StartMissionInput};
use runner_core::protocol::project::ProjectScope;
use runner_core::protocol::worktree::WorktreeSpec;
use runner_core::protocol::StartDirectSessionArgs;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{mpsc, Arc, Mutex};

#[derive(Default)]
struct MockRuntime {
    spawns: Mutex<Vec<SpawnSpec>>,
    outputs: Mutex<HashMap<String, mpsc::Sender<crate::session::runtime::RuntimeOutput>>>,
}

impl SessionRuntime for MockRuntime {
    fn spawn(&self, spec: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)> {
        self.spawns.lock().unwrap().push(spec.clone());
        let (sender, receiver) = mpsc::channel();
        self.outputs
            .lock()
            .unwrap()
            .insert(spec.session_id.clone(), sender);
        let session = RuntimeSession {
            runtime: "tracking".into(),
            session_id: spec.session_id,
        };
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        Ok((session, OutputStream::new(receiver, stop)))
    }

    fn stop(&self, session: &RuntimeSession) -> RuntimeResult<()> {
        self.outputs.lock().unwrap().remove(&session.session_id);
        Ok(())
    }

    fn send_bytes(&self, _session: &RuntimeSession, _bytes: &[u8]) -> RuntimeResult<()> {
        Ok(())
    }

    fn send_key(&self, _session: &RuntimeSession, _key: &str) -> RuntimeResult<()> {
        Ok(())
    }

    fn resize(&self, _session: &RuntimeSession, _cols: u16, _rows: u16) -> RuntimeResult<()> {
        Ok(())
    }

    fn status(&self, _session: &RuntimeSession) -> RuntimeResult<Option<PtyStatus>> {
        Ok(Some(PtyStatus {
            alive: false,
            exit_code: Some(0),
            ..Default::default()
        }))
    }
}

struct FailingRuntime;

impl SessionRuntime for FailingRuntime {
    fn spawn(&self, _spec: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)> {
        Err(RuntimeError::Msg("intentional spawn failure".into()))
    }

    fn stop(&self, _session: &RuntimeSession) -> RuntimeResult<()> {
        Ok(())
    }

    fn send_bytes(&self, _session: &RuntimeSession, _bytes: &[u8]) -> RuntimeResult<()> {
        Ok(())
    }

    fn send_key(&self, _session: &RuntimeSession, _key: &str) -> RuntimeResult<()> {
        Ok(())
    }

    fn resize(&self, _session: &RuntimeSession, _cols: u16, _rows: u16) -> RuntimeResult<()> {
        Ok(())
    }

    fn status(&self, _session: &RuntimeSession) -> RuntimeResult<Option<PtyStatus>> {
        Ok(None)
    }
}

fn core_with_mock_runtime(app_data_dir: &Path) -> AppCore {
    let mut core = test_core_in(app_data_dir.to_path_buf());
    core.db = Arc::new(crate::db::open_pool(&app_data_dir.join("runner.db")).unwrap());
    core.sessions = SessionManager::new(
        Arc::clone(&core.runtime_shell_env),
        Arc::clone(&core.runtime_discovery),
        Arc::new(MockRuntime::default()),
    );
    core
}

fn core_with_failing_runtime(app_data_dir: &Path) -> AppCore {
    let mut core = test_core_in(app_data_dir.to_path_buf());
    core.sessions = SessionManager::new(
        Arc::clone(&core.runtime_shell_env),
        Arc::clone(&core.runtime_discovery),
        Arc::new(FailingRuntime),
    );
    core
}

fn init_repo(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "core.autocrlf", "false"]);
    std::fs::write(dir.join("README.md"), "hello\n").unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "init"]);
}

fn seed_role_and_crew(core: &AppCore) -> (String, String) {
    let conn = core.db.get().unwrap();
    let exe = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    insert_test_role(&conn, "r1", "coder", "test", &exe);
    insert_test_role(&conn, "r2", "reviewer", "test", &exe);
    conn.execute(
        "INSERT INTO crews (id, name, created_at, updated_at)
         VALUES ('c1', 'Trio', '2026-10-01T00:00:00Z', '2026-10-01T00:00:00Z')",
        [],
    )
    .unwrap();
    insert_test_slot(&conn, "sl1", "c1", "r1", "coder", 0, true);
    ("r1".into(), "c1".into())
}

#[test]
fn git_aware_project_membership_and_precedence_across_chat_terminal_and_mission() {
    let root = tempfile::Builder::new()
        .prefix("1213-m1-member-")
        .tempdir()
        .unwrap();
    let app_data = root.path().join("app-data");
    let main_repo = root.path().join("main-repo");
    let sibling = root.path().join("sibling-non-git");
    let ext_wt = root.path().join("codex-external-wt");
    let ext_sub = ext_wt.join("nested").join("sub");
    std::fs::create_dir_all(&app_data).unwrap();
    std::fs::create_dir_all(&main_repo).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    init_repo(&main_repo);
    git(
        &main_repo,
        &[
            "worktree",
            "add",
            "-b",
            "feat/external",
            &ext_wt.to_string_lossy(),
            "main",
        ],
    );
    std::fs::create_dir_all(&ext_sub).unwrap();

    let dedicated_wt = normalize_checkout_path(&main_repo)
        .join(".worktrees")
        .join("feat-dedicated");
    git(
        &main_repo,
        &[
            "worktree",
            "add",
            "-b",
            "feat/dedicated",
            &dedicated_wt.to_string_lossy(),
            "main",
        ],
    );

    let core = core_with_mock_runtime(&app_data);
    let (role_id, crew_id) = seed_role_and_crew(&core);
    let main_project = {
        let conn = core.db.get().unwrap();
        repo::project::create(&conn, "Main", &main_repo.to_string_lossy()).unwrap()
    };
    let dedicated_project = {
        let conn = core.db.get().unwrap();
        repo::project::create(&conn, "Dedicated", &dedicated_wt.to_string_lossy()).unwrap()
    };

    let git_handle = core.files.git();
    let conn = core.db.get().unwrap();
    // Containment & sibling non-git behavior.
    assert_eq!(
        project::resolve_scope_with_git(
            &conn,
            git_handle.as_ref(),
            &ProjectScope::Infer,
            Some(main_repo.to_string_lossy().into_owned()),
        )
        .unwrap()
        .0
        .as_deref(),
        Some(main_project.id.as_str())
    );
    assert_eq!(
        project::resolve_scope_with_git(
            &conn,
            git_handle.as_ref(),
            &ProjectScope::Infer,
            Some(sibling.to_string_lossy().into_owned()),
        )
        .unwrap()
        .0,
        None
    );
    // Dedicated registered worktree keeps its own project rather than falling back to Main.
    assert_eq!(
        project::resolve_scope_with_git(
            &conn,
            git_handle.as_ref(),
            &ProjectScope::Infer,
            Some(dedicated_wt.to_string_lossy().into_owned()),
        )
        .unwrap()
        .0
        .as_deref(),
        Some(dedicated_project.id.as_str())
    );
    // Root keeps worktree outside projects.
    assert_eq!(
        project::resolve_scope_with_git(
            &conn,
            git_handle.as_ref(),
            &ProjectScope::Root,
            Some(ext_wt.to_string_lossy().into_owned()),
        )
        .unwrap()
        .0,
        None
    );
    drop(conn);

    // Chat in subfolder of an external linked worktree infers Main via git.
    let chat = session::session_start_chat(
        &core,
        StartDirectSessionArgs {
            role_id: Some(role_id),
            cwd: Some(ext_sub.to_string_lossy().into_owned()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(chat.project_id.as_deref(), Some(main_project.id.as_str()));
    session::session_kill(&core, &chat.session.id).unwrap();

    // Terminal in external linked worktree infers Main via git.
    let shell = session::session_start_shell_in(
        &core,
        ProjectScope::Infer,
        Some(ext_wt.to_string_lossy().into_owned()),
        None,
        None,
        None,
    )
    .unwrap();
    let shell_row = repo::session::get_row(&core.db.get().unwrap(), &shell.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        shell_row.project_id.as_deref(),
        Some(main_project.id.as_str())
    );
    session::session_kill(&core, &shell.id).unwrap();

    // Mission started in external linked worktree infers Main via git.
    let started = crate::daemon::block_on(mission::mission_start_impl_with_size(
        &core,
        StartMissionInput {
            crew_id: Some(crew_id),
            project_id: None,
            title: "External worktree mission".into(),
            goal_override: Some("Check membership".into()),
            cwd: Some(ext_sub.to_string_lossy().into_owned()),
            worktree: None,
            started_by_session_id: None,
            no_report: false,
        }
        .into(),
        None,
    ))
    .unwrap();
    assert_eq!(
        started.mission.project_id.as_deref(),
        Some(main_project.id.as_str())
    );
    crate::daemon::block_on(mission::mission_stop_impl(&core, started.mission.id)).unwrap();
}

#[test]
fn create_and_start_idempotent_failed_add_and_spawn_failure_retention() {
    let root = tempfile::Builder::new()
        .prefix("1213-m1-start-")
        .tempdir()
        .unwrap();
    let app_data = root.path().join("app-data");
    let main_repo = root.path().join("repo");
    std::fs::create_dir_all(&app_data).unwrap();
    std::fs::create_dir_all(&main_repo).unwrap();
    init_repo(&main_repo);

    let core = core_with_mock_runtime(&app_data);
    let (role_id, crew_id) = seed_role_and_crew(&core);
    let project = {
        let conn = core.db.get().unwrap();
        repo::project::create(&conn, "Repo", &main_repo.to_string_lossy()).unwrap()
    };

    // Failed add records nothing and spawns nothing across chat, terminal, and mission.
    let bad_wt = WorktreeSpec {
        branch: "bad..branch".into(),
        base: None,
    };
    assert!(session::session_start_chat(
        &core,
        StartDirectSessionArgs {
            role_id: Some(role_id.clone()),
            project_id: Some(project.id.clone()),
            worktree: Some(bad_wt.clone()),
            ..Default::default()
        }
    )
    .is_err());
    assert!(session::session_start_shell_in(
        &core,
        ProjectScope::Project(project.id.clone()),
        None,
        None,
        None,
        Some(bad_wt.clone()),
    )
    .is_err());
    assert!(
        crate::daemon::block_on(mission::mission_start_impl_with_size(
            &core,
            MissionStart {
                crew_id: Some(crew_id.clone()),
                scope: ProjectScope::Project(project.id.clone()),
                title: "Should fail".into(),
                goal_override: Some("No side effects".into()),
                cwd: None,
                worktree: Some(bad_wt),
                started_by_session_id: None,
                no_report: false,
            },
            None,
        ))
        .is_err()
    );
    {
        let conn = core.db.get().unwrap();
        assert_eq!(row_count(&conn, "SELECT COUNT(*) FROM sessions"), 0);
        assert_eq!(row_count(&conn, "SELECT COUNT(*) FROM missions"), 0);
    }

    // Valid chat create-and-start and idempotent retry on the same branch.
    let wt_spec = WorktreeSpec {
        branch: "feat/chat-wt".into(),
        base: Some("main".into()),
    };
    let expected_chat_wt = normalize_checkout_path(&main_repo)
        .join(".worktrees")
        .join("feat-chat-wt");
    let chat1 = session::session_start_chat(
        &core,
        StartDirectSessionArgs {
            role_id: Some(role_id.clone()),
            project_id: Some(project.id.clone()),
            worktree: Some(wt_spec.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(expected_chat_wt.is_dir());
    assert!(same_normalized_checkout_path(
        Path::new(chat1.cwd.as_deref().unwrap()),
        &expected_chat_wt
    ));
    assert_eq!(chat1.project_id.as_deref(), Some(project.id.as_str()));

    let chat2 = session::session_start_chat(
        &core,
        StartDirectSessionArgs {
            role_id: Some(role_id.clone()),
            project_id: Some(project.id.clone()),
            worktree: Some(wt_spec),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(same_normalized_checkout_path(
        Path::new(chat2.cwd.as_deref().unwrap()),
        &expected_chat_wt
    ));
    session::session_kill(&core, &chat1.session.id).unwrap();
    session::session_kill(&core, &chat2.session.id).unwrap();

    // Terminal create-and-start.
    let shell = session::session_start_shell_in(
        &core,
        ProjectScope::Project(project.id.clone()),
        None,
        None,
        None,
        Some(WorktreeSpec {
            branch: "feat/shell-wt".into(),
            base: None,
        }),
    )
    .unwrap();
    let expected_shell_wt = normalize_checkout_path(&main_repo)
        .join(".worktrees")
        .join("feat-shell-wt");
    assert!(expected_shell_wt.is_dir());
    let shell_row = repo::session::get_row(&core.db.get().unwrap(), &shell.id)
        .unwrap()
        .unwrap();
    assert!(same_normalized_checkout_path(
        Path::new(shell_row.cwd.as_deref().unwrap()),
        &expected_shell_wt
    ));
    session::session_kill(&core, &shell.id).unwrap();

    // Mission create-and-start with retry reusing the same worktree.
    let expected_mission_wt = normalize_checkout_path(&main_repo)
        .join(".worktrees")
        .join("feat-mission-wt");
    let m1 = crate::daemon::block_on(mission::mission_start_impl_with_size(
        &core,
        MissionStart {
            crew_id: Some(crew_id.clone()),
            scope: ProjectScope::Project(project.id.clone()),
            title: "First mission".into(),
            goal_override: Some("Work in wt".into()),
            cwd: None,
            worktree: Some(WorktreeSpec {
                branch: "feat/mission-wt".into(),
                base: None,
            }),
            started_by_session_id: None,
            no_report: false,
        },
        None,
    ))
    .unwrap();
    assert!(expected_mission_wt.is_dir());
    assert!(same_normalized_checkout_path(
        Path::new(m1.mission.cwd.as_deref().unwrap()),
        &expected_mission_wt
    ));
    crate::daemon::block_on(mission::mission_stop_impl(&core, m1.mission.id)).unwrap();

    // Spawn failure after add keeps the newly created worktree.
    let fail_app = root.path().join("app-data-fail");
    std::fs::create_dir_all(&fail_app).unwrap();
    let failing = core_with_failing_runtime(&fail_app);
    let _ = seed_role_and_crew(&failing);
    let fail_project = {
        let conn = failing.db.get().unwrap();
        repo::project::create(&conn, "Repo", &main_repo.to_string_lossy()).unwrap()
    };
    let expected_fail_chat = normalize_checkout_path(&main_repo)
        .join(".worktrees")
        .join("feat-fail-chat");
    assert!(session::session_start_chat(
        &failing,
        StartDirectSessionArgs {
            role_id: Some("r1".into()),
            project_id: Some(fail_project.id.clone()),
            worktree: Some(WorktreeSpec {
                branch: "feat/fail-chat".into(),
                base: None,
            }),
            ..Default::default()
        },
    )
    .is_err());
    assert!(expected_fail_chat.is_dir());

    let expected_fail_shell = normalize_checkout_path(&main_repo)
        .join(".worktrees")
        .join("feat-fail-shell");
    assert!(session::session_start_shell_in(
        &failing,
        ProjectScope::Project(fail_project.id.clone()),
        None,
        None,
        None,
        Some(WorktreeSpec {
            branch: "feat/fail-shell".into(),
            base: None,
        }),
    )
    .is_err());
    assert!(expected_fail_shell.is_dir());

    // Fail mission launch after `start_with_files` creates the worktree (oversized role prompt).
    let failing_mission_crew = {
        let conn = failing.db.get().unwrap();
        let exe = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        insert_test_role(&conn, "r-huge", "huge", "test", &exe);
        let huge_prompt = "x".repeat(200_000);
        conn.execute(
            "UPDATE roles SET system_prompt = ?2 WHERE id = ?1",
            rusqlite::params!["r-huge", huge_prompt],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES ('c-broken', 'Broken', '2026-10-01T00:00:00Z', '2026-10-01T00:00:00Z')",
            [],
        )
        .unwrap();
        insert_test_slot(&conn, "sl-broken", "c-broken", "r-huge", "huge", 0, true);
        "c-broken".to_owned()
    };
    let expected_fail_mission = normalize_checkout_path(&main_repo)
        .join(".worktrees")
        .join("feat-fail-mission");
    assert!(
        crate::daemon::block_on(mission::mission_start_impl_with_size(
            &failing,
            MissionStart {
                crew_id: Some(failing_mission_crew),
                scope: ProjectScope::Project(fail_project.id),
                title: "Spawn fails".into(),
                goal_override: Some("Keep wt".into()),
                cwd: None,
                worktree: Some(WorktreeSpec {
                    branch: "feat/fail-mission".into(),
                    base: None,
                }),
                started_by_session_id: None,
                no_report: false,
            },
            None,
        ))
        .is_err()
    );
    assert!(expected_fail_mission.is_dir());
}

#[test]
fn restore_unarchive_resume_restart_add_role_and_boot_resume_on_missing_worktree() {
    let root = tempfile::Builder::new()
        .prefix("1213-m1-lifecycle-")
        .tempdir()
        .unwrap();
    let app_data = root.path().join("app-data");
    let main_repo = root.path().join("repo");
    std::fs::create_dir_all(&app_data).unwrap();
    std::fs::create_dir_all(&main_repo).unwrap();
    init_repo(&main_repo);

    let core = core_with_mock_runtime(&app_data);
    let (role_id, crew_id) = seed_role_and_crew(&core);
    let project = {
        let conn = core.db.get().unwrap();
        repo::project::create(&conn, "Repo", &main_repo.to_string_lossy()).unwrap()
    };

    // Start a chat, a terminal shell, and a mission in linked worktrees, then stop them.
    let chat = session::session_start_chat(
        &core,
        StartDirectSessionArgs {
            role_id: Some(role_id),
            project_id: Some(project.id.clone()),
            worktree: Some(WorktreeSpec {
                branch: "feat/chat-rec".into(),
                base: None,
            }),
            ..Default::default()
        },
    )
    .unwrap();
    let chat_wt = normalize_checkout_path(&main_repo)
        .join(".worktrees")
        .join("feat-chat-rec");

    let shell = session::session_start_shell_in(
        &core,
        ProjectScope::Project(project.id.clone()),
        None,
        None,
        None,
        Some(WorktreeSpec {
            branch: "feat/shell-rec".into(),
            base: None,
        }),
    )
    .unwrap();
    let shell_wt = normalize_checkout_path(&main_repo)
        .join(".worktrees")
        .join("feat-shell-rec");

    let started = crate::daemon::block_on(mission::mission_start_impl_with_size(
        &core,
        MissionStart {
            crew_id: Some(crew_id),
            scope: ProjectScope::Project(project.id.clone()),
            title: "Mission recover".into(),
            goal_override: Some("Test recover".into()),
            cwd: None,
            worktree: Some(WorktreeSpec {
                branch: "feat/mission-rec".into(),
                base: None,
            }),
            started_by_session_id: None,
            no_report: false,
        },
        None,
    ))
    .unwrap();
    let mission_wt = normalize_checkout_path(&main_repo)
        .join(".worktrees")
        .join("feat-mission-rec");
    let mission_slot_id = session::session_list(&core, &started.mission.id).unwrap()[0]
        .session
        .id
        .clone();

    // Mark running sessions resumable on launch (exact shutdown sequence so exiting shells keep their row)
    // and then stop them.
    {
        let mut conn = core.db.get().unwrap();
        for (idx, id) in [&chat.session.id, &mission_slot_id].into_iter().enumerate() {
            conn.execute(
                "UPDATE sessions
                    SET status = 'running',
                        agent_session_key = COALESCE(agent_session_key, ?2)
                  WHERE id = ?1",
                rusqlite::params![id, format!("key-{idx}")],
            )
            .unwrap();
        }
        repo::session::mark_running_for_resume_on_launch(&mut conn).unwrap();
    }
    session::session_stop(&core, &chat.session.id).unwrap();
    session::session_stop(&core, &shell.id).unwrap();
    crate::daemon::block_on(mission::mission_stop_impl(
        &core,
        started.mission.id.clone(),
    ))
    .unwrap();
    {
        let conn = core.db.get().unwrap();
        for id in [&chat.session.id, &shell.id, &mission_slot_id] {
            conn.execute("UPDATE sessions SET status = 'stopped' WHERE id = ?1", [id])
                .unwrap();
        }
    }

    // Delete all three worktrees externally (`git worktree remove` + `rm -rf`).
    git(
        &main_repo,
        &["worktree", "remove", &chat_wt.to_string_lossy()],
    );
    std::fs::remove_dir_all(&shell_wt).unwrap();
    git(
        &main_repo,
        &["worktree", "remove", &mission_wt.to_string_lossy()],
    );
    assert!(!chat_wt.exists());
    assert!(!shell_wt.exists());
    assert!(!mission_wt.exists());

    // Automatic boot resume MUST NOT recreate any missing worktree and MUST keep rows stopped.
    let report = crate::daemon::resume::consume_resume_on_launch_until(
        &core,
        true,
        |_| Some((80, 24)),
        &crate::daemon::resume::ResumeConsumer::default(),
    )
    .unwrap();
    assert!(report.resumed.is_empty());
    assert_eq!(report.errors.len(), 3, "errors: {:?}", report.errors);
    assert!(!chat_wt.exists());
    assert!(!shell_wt.exists());
    assert!(!mission_wt.exists());
    {
        let conn = core.db.get().unwrap();
        for id in [&chat.session.id, &shell.id, &mission_slot_id] {
            let row = repo::session::get_row(&conn, id).unwrap().unwrap();
            assert_eq!(row.status, SessionStatus::Stopped);
        }
    }

    // Explicit `session_resume` recreates the missing chat and shell worktrees.
    session::session_resume(&core, &chat.session.id, None, None).unwrap();
    assert!(chat_wt.is_dir());
    session::session_resume(&core, &shell.id, None, None).unwrap();
    assert!(shell_wt.is_dir());

    // `mission_resume_with_size` recreates the missing mission worktree before resuming its slot.
    mission::mission_resume_with_size(&core, &started.mission.id, None).unwrap();
    assert!(mission_wt.is_dir());

    // `session_restart` recreates if missing before restarting the slot.
    crate::daemon::block_on(mission::mission_stop_impl(
        &core,
        started.mission.id.clone(),
    ))
    .unwrap();
    std::fs::remove_dir_all(&mission_wt).unwrap();
    assert!(!mission_wt.exists());
    mission::mission_resume_with_size(&core, &started.mission.id, None).unwrap();
    assert!(mission_wt.is_dir());
    std::fs::remove_dir_all(&mission_wt).unwrap();
    // Restart fails when stopped/only running slots? Wait: restart runs on a running or stopped slot.
    crate::daemon::block_on(mission::ensure_mission_router_mounted(
        &core,
        &started.mission.id,
    ))
    .unwrap();
    session::session_restart(&core, &mission_slot_id, None, None).unwrap();
    assert!(mission_wt.is_dir());

    // `mission_add_role` recreates missing mission worktree before registering the new role.
    std::fs::remove_dir_all(&mission_wt).unwrap();
    mission::roles::add_role(
        &core,
        MissionAddRoleInput {
            mission_id: started.mission.id.clone(),
            role_id: "r2".into(),
            handle: None,
            runtime: None,
            model: None,
            effort: None,
            speed: None,
            task: None,
        },
    )
    .unwrap();
    assert!(mission_wt.is_dir());

    // Archive chat & mission, remove worktrees, then test `session_unarchive` and `mission_unarchive_impl`.
    session::session_archive_direct(&core, &chat.session.id).unwrap();
    crate::daemon::block_on(mission::mission_stop_impl(
        &core,
        started.mission.id.clone(),
    ))
    .unwrap();
    crate::daemon::block_on(mission::mission_archive_impl(
        &core,
        started.mission.id.clone(),
    ))
    .unwrap();
    git(
        &main_repo,
        &["worktree", "remove", &chat_wt.to_string_lossy()],
    );
    git(
        &main_repo,
        &["worktree", "remove", &mission_wt.to_string_lossy()],
    );

    session::session_unarchive(&core, &chat.session.id).unwrap();
    assert!(chat_wt.is_dir());
    crate::daemon::block_on(mission::mission_unarchive_impl(
        &core,
        started.mission.id.clone(),
    ))
    .unwrap();
    assert!(mission_wt.is_dir());

    // Missing branch returns the Restore-in-main-checkout fallback error and leaves chat archived.
    session::session_archive_direct(&core, &chat.session.id).unwrap();
    git(
        &main_repo,
        &["worktree", "remove", &chat_wt.to_string_lossy()],
    );
    git(&main_repo, &["branch", "-D", "feat/chat-rec"]);
    let err = session::session_unarchive(&core, &chat.session.id)
        .unwrap_err()
        .to_string();
    assert!(err.contains("restore in main checkout"), "got: {err}");
    assert!(
        repo::session::get_row(&core.db.get().unwrap(), &chat.session.id)
            .unwrap()
            .unwrap()
            .archived_at
            .is_some()
    );
}

#[test]
fn worktree_remove_synchronizes_with_concurrent_starts_and_unarchive() {
    let root = tempfile::Builder::new()
        .prefix("1213-m1-race-")
        .tempdir()
        .unwrap();
    let app_data = root.path().join("app-data");
    let main_repo = root.path().join("repo");
    std::fs::create_dir_all(&app_data).unwrap();
    std::fs::create_dir_all(&main_repo).unwrap();
    init_repo(&main_repo);

    let core = Arc::new(core_with_mock_runtime(&app_data));
    let (role_id, crew_id) = seed_role_and_crew(&core);
    let project = {
        let conn = core.db.get().unwrap();
        repo::project::create(&conn, "Repo", &main_repo.to_string_lossy()).unwrap()
    };

    // 1. `--worktree` chat start holds gate until session row inserts; concurrent remove waits and refuses.
    {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (proceed_tx, proceed_rx) = mpsc::channel();
        let (contended_tx, contended_rx) = mpsc::channel();
        core.files
            .worktree_gate()
            .pause_next_registration(ready_tx, proceed_rx);
        core.files
            .worktree_gate()
            .notify_write_contended(contended_tx);

        let core_start = Arc::clone(&core);
        let project_id = project.id.clone();
        let role_id = role_id.clone();
        let start_handle = std::thread::spawn(move || {
            session::session_start_chat(
                &core_start,
                StartDirectSessionArgs {
                    role_id: Some(role_id),
                    project_id: Some(project_id),
                    worktree: Some(WorktreeSpec {
                        branch: "feat/race-wt".into(),
                        base: None,
                    }),
                    ..Default::default()
                },
            )
        });

        ready_rx.recv().unwrap();
        let core_remove = Arc::clone(&core);
        let repo_remove = main_repo.clone();
        let remove_handle = std::thread::spawn(move || {
            crate::files::worktree_remove(&core_remove, &repo_remove, "feat/race-wt")
        });

        contended_rx.recv().unwrap();
        proceed_tx.send(()).unwrap();

        let started = start_handle.join().unwrap().unwrap();
        let remove_err = remove_handle.join().unwrap().unwrap_err().to_string();
        assert!(remove_err.contains("in use by"), "got: {remove_err}");
        let wt_path = normalize_checkout_path(&main_repo)
            .join(".worktrees")
            .join("feat-race-wt");
        assert!(wt_path.is_dir());
        session::session_archive_direct(&core, &started.session.id).unwrap();
    }

    // 2. Plain `--cwd` chat start holds gate until row inserts; concurrent remove waits and refuses.
    let race_cwd_wt = core
        .files
        .worktree_create(&main_repo, "feat/race-cwd", None)
        .unwrap()
        .path;
    let cwd_session_id = {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (proceed_tx, proceed_rx) = mpsc::channel();
        let (contended_tx, contended_rx) = mpsc::channel();
        core.files
            .worktree_gate()
            .pause_next_registration(ready_tx, proceed_rx);
        core.files
            .worktree_gate()
            .notify_write_contended(contended_tx);

        let core_start = Arc::clone(&core);
        let role_id = role_id.clone();
        let cwd_str = race_cwd_wt.to_string_lossy().into_owned();
        let start_handle = std::thread::spawn(move || {
            session::session_start_chat(
                &core_start,
                StartDirectSessionArgs {
                    role_id: Some(role_id),
                    cwd: Some(cwd_str),
                    ..Default::default()
                },
            )
        });

        ready_rx.recv().unwrap();
        let core_remove = Arc::clone(&core);
        let repo_remove = main_repo.clone();
        let remove_handle = std::thread::spawn(move || {
            crate::files::worktree_remove(&core_remove, &repo_remove, "feat/race-cwd")
        });

        contended_rx.recv().unwrap();
        proceed_tx.send(()).unwrap();

        let started = start_handle.join().unwrap().unwrap();
        let remove_err = remove_handle.join().unwrap().unwrap_err().to_string();
        assert!(remove_err.contains("in use by"), "got: {remove_err}");
        assert!(race_cwd_wt.is_dir());
        session::session_archive_direct(&core, &started.session.id).unwrap();
        started.session.id
    };

    // 3. `session_unarchive` holds gate until unarchived in DB; concurrent remove waits and refuses.
    {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (proceed_tx, proceed_rx) = mpsc::channel();
        let (contended_tx, contended_rx) = mpsc::channel();
        core.files
            .worktree_gate()
            .pause_next_registration(ready_tx, proceed_rx);
        core.files
            .worktree_gate()
            .notify_write_contended(contended_tx);

        let core_unarchive = Arc::clone(&core);
        let sid = cwd_session_id.clone();
        let unarchive_handle =
            std::thread::spawn(move || session::session_unarchive(&core_unarchive, &sid));

        ready_rx.recv().unwrap();
        let core_remove = Arc::clone(&core);
        let repo_remove = main_repo.clone();
        let remove_handle = std::thread::spawn(move || {
            crate::files::worktree_remove(&core_remove, &repo_remove, "feat/race-cwd")
        });

        contended_rx.recv().unwrap();
        proceed_tx.send(()).unwrap();

        unarchive_handle.join().unwrap().unwrap();
        let remove_err = remove_handle.join().unwrap().unwrap_err().to_string();
        assert!(remove_err.contains("in use by"), "got: {remove_err}");
        assert!(race_cwd_wt.is_dir());
        session::session_archive_direct(&core, &cwd_session_id).unwrap();
    }

    // 4. Reverse interleaving: `worktree_remove` deletes first while concurrent `--cwd` start waits and fails without inserting a session.
    {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (proceed_tx, proceed_rx) = mpsc::channel();
        let (contended_tx, contended_rx) = mpsc::channel();
        core.files
            .worktree_gate()
            .pause_next_git_remove(ready_tx, proceed_rx);
        core.files
            .worktree_gate()
            .notify_read_contended(contended_tx);

        let core_remove = Arc::clone(&core);
        let repo_remove = main_repo.clone();
        let remove_handle = std::thread::spawn(move || {
            crate::files::worktree_remove(&core_remove, &repo_remove, "feat/race-cwd")
        });

        ready_rx.recv().unwrap();
        let core_start = Arc::clone(&core);
        let role_id = role_id.clone();
        let cwd_str = race_cwd_wt.to_string_lossy().into_owned();
        let before_count = row_count(&core.db.get().unwrap(), "SELECT COUNT(*) FROM sessions");
        let start_handle = std::thread::spawn(move || {
            session::session_start_chat(
                &core_start,
                StartDirectSessionArgs {
                    role_id: Some(role_id),
                    cwd: Some(cwd_str),
                    ..Default::default()
                },
            )
        });

        contended_rx.recv().unwrap();
        proceed_tx.send(()).unwrap();

        remove_handle.join().unwrap().unwrap();
        let start_err = start_handle.join().unwrap().unwrap_err().to_string();
        assert!(
            start_err.contains("working directory does not exist"),
            "got: {start_err}"
        );
        assert!(!race_cwd_wt.exists());
        assert_eq!(
            row_count(&core.db.get().unwrap(), "SELECT COUNT(*) FROM sessions"),
            before_count
        );
    }

    // 5. Mission `--worktree` start also holds gate across create + mission row insert.
    {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (proceed_tx, proceed_rx) = mpsc::channel();
        let (contended_tx, contended_rx) = mpsc::channel();
        core.files
            .worktree_gate()
            .pause_next_registration(ready_tx, proceed_rx);
        core.files
            .worktree_gate()
            .notify_write_contended(contended_tx);

        let core_mission = Arc::clone(&core);
        let project_id = project.id.clone();
        let crew_id = crew_id.clone();
        let mission_handle = std::thread::spawn(move || {
            crate::daemon::block_on(mission::mission_start_impl_with_size(
                &core_mission,
                MissionStart {
                    crew_id: Some(crew_id),
                    scope: ProjectScope::Project(project_id),
                    title: "Race mission".into(),
                    goal_override: Some("Test".into()),
                    cwd: None,
                    worktree: Some(WorktreeSpec {
                        branch: "feat/race-mission".into(),
                        base: None,
                    }),
                    started_by_session_id: None,
                    no_report: false,
                },
                None,
            ))
        });

        ready_rx.recv().unwrap();
        let core_remove = Arc::clone(&core);
        let repo_remove = main_repo.clone();
        let remove_handle = std::thread::spawn(move || {
            crate::files::worktree_remove(&core_remove, &repo_remove, "feat/race-mission")
        });

        contended_rx.recv().unwrap();
        proceed_tx.send(()).unwrap();

        let started = mission_handle.join().unwrap().unwrap();
        let remove_err = remove_handle.join().unwrap().unwrap_err().to_string();
        assert!(remove_err.contains("in use by"), "got: {remove_err}");
        let mwt = normalize_checkout_path(&main_repo)
            .join(".worktrees")
            .join("feat-race-mission");
        assert!(mwt.is_dir());
        crate::daemon::block_on(mission::mission_stop_impl(&core, started.mission.id)).unwrap();
    }
}

#[test]
fn external_worktree_with_and_without_prunable_entry_restores_and_blocks_boot_resume() {
    let root = tempfile::Builder::new()
        .prefix("1213-m1-ext-lifecycle-")
        .tempdir()
        .unwrap();
    let app_data = root.path().join("app-data");
    let main_repo = root.path().join("repo");
    let outside_wt = root.path().join("feat-outside");
    std::fs::create_dir_all(&app_data).unwrap();
    std::fs::create_dir_all(&main_repo).unwrap();
    init_repo(&main_repo);
    git(
        &main_repo,
        &[
            "worktree",
            "add",
            "-b",
            "feat/outside",
            &outside_wt.to_string_lossy(),
            "main",
        ],
    );

    let core = core_with_mock_runtime(&app_data);
    let (role_id, _crew_id) = seed_role_and_crew(&core);
    let project = {
        let conn = core.db.get().unwrap();
        repo::project::create(&conn, "Repo", &main_repo.to_string_lossy()).unwrap()
    };

    let outside_str = outside_wt.to_string_lossy().into_owned();
    let chat = session::session_start_chat(
        &core,
        StartDirectSessionArgs {
            role_id: Some(role_id.clone()),
            cwd: Some(outside_str.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(chat.project_id.as_deref(), Some(project.id.as_str()));

    let shell = session::session_start_shell_in(
        &core,
        ProjectScope::Infer,
        Some(outside_str.clone()),
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        repo::session::get_row(&core.db.get().unwrap(), &shell.id)
            .unwrap()
            .unwrap()
            .project_id
            .as_deref(),
        Some(project.id.as_str())
    );

    let mark_resumable_and_stop = || {
        {
            let mut conn = core.db.get().unwrap();
            conn.execute(
                "UPDATE sessions
                    SET status = 'running',
                        agent_session_key = COALESCE(agent_session_key, 'key-ext')
                  WHERE id = ?1",
                [&chat.session.id],
            )
            .unwrap();
            conn.execute(
                "UPDATE sessions SET status = 'running' WHERE id = ?1",
                [&shell.id],
            )
            .unwrap();
            repo::session::mark_running_for_resume_on_launch(&mut conn).unwrap();
        }
        session::session_stop(&core, &chat.session.id).unwrap();
        session::session_stop(&core, &shell.id).unwrap();
        let conn = core.db.get().unwrap();
        for id in [&chat.session.id, &shell.id] {
            conn.execute("UPDATE sessions SET status = 'stopped' WHERE id = ?1", [id])
                .unwrap();
        }
    };

    // 1. Deleted manually (`rm -rf ../feat-outside`) -> prunable entry present.
    mark_resumable_and_stop();
    std::fs::remove_dir_all(&outside_wt).unwrap();
    let checkout_prunable =
        crate::files::worktree_checkout(&core, &outside_wt, Some(&project.id)).unwrap();
    assert!(matches!(
        checkout_prunable,
        runner_core::protocol::worktree::WorktreeCheckout::Git {
            managed: false,
            branch: Some(ref b),
            missing: true,
            missing_branch: Some(runner_core::protocol::worktree::MissingWorktreeBranchState::Surviving),
            ..
        } if b == "feat/outside"
    ));

    let report_prunable = crate::daemon::resume::consume_resume_on_launch_until(
        &core,
        true,
        |_| Some((80, 24)),
        &crate::daemon::resume::ResumeConsumer::default(),
    )
    .unwrap();
    assert!(report_prunable.resumed.is_empty());
    assert_eq!(report_prunable.errors.len(), 2);
    assert!(!outside_wt.exists());
    for id in [&chat.session.id, &shell.id] {
        let row = repo::session::get_row(&core.db.get().unwrap(), id)
            .unwrap()
            .unwrap();
        assert_eq!(row.status, SessionStatus::Stopped);
    }

    // Explicit resume recreates the external worktree from prunable registration.
    session::session_resume(&core, &chat.session.id, None, None).unwrap();
    assert!(outside_wt.is_dir());
    session::session_resume(&core, &shell.id, None, None).unwrap();

    // 2. Removed with `git worktree remove ../feat-outside` (NO prunable entry left).
    mark_resumable_and_stop();
    git(&main_repo, &["worktree", "remove", &outside_str]);
    assert!(!outside_wt.exists());
    let listed = core.files.worktree_list(&main_repo).unwrap();
    assert_eq!(listed.worktrees.len(), 1);

    let checkout_unpruned =
        crate::files::worktree_checkout(&core, &outside_wt, Some(&project.id)).unwrap();
    assert!(matches!(
        checkout_unpruned,
        runner_core::protocol::worktree::WorktreeCheckout::Git {
            managed: false,
            branch: Some(ref b),
            missing: true,
            missing_branch: Some(runner_core::protocol::worktree::MissingWorktreeBranchState::Surviving),
            ..
        } if b == "feat/outside"
    ));

    let report_removed = crate::daemon::resume::consume_resume_on_launch_until(
        &core,
        true,
        |_| Some((80, 24)),
        &crate::daemon::resume::ResumeConsumer::default(),
    )
    .unwrap();
    assert!(report_removed.resumed.is_empty());
    assert_eq!(report_removed.errors.len(), 2);
    assert!(!outside_wt.exists());
    for id in [&chat.session.id, &shell.id] {
        let row = repo::session::get_row(&core.db.get().unwrap(), id)
            .unwrap()
            .unwrap();
        assert_eq!(row.status, SessionStatus::Stopped);
    }

    // Explicit resume recreates `../feat-outside` cleanly via unique flattened match.
    session::session_resume(&core, &chat.session.id, None, None).unwrap();
    assert!(outside_wt.is_dir());
    assert_eq!(
        git(&outside_wt, &["branch", "--show-current"]).trim(),
        "feat/outside"
    );
    session::session_resume(&core, &shell.id, None, None).unwrap();

    // 3. Ambiguous branch match preserves restore fallback error and blocks boot resume.
    mark_resumable_and_stop();
    git(&main_repo, &["branch", "feat-outside", "main"]);
    git(&main_repo, &["worktree", "remove", &outside_str]);

    let ambig_report = crate::daemon::resume::consume_resume_on_launch_until(
        &core,
        true,
        |_| Some((80, 24)),
        &crate::daemon::resume::ResumeConsumer::default(),
    )
    .unwrap();
    assert!(ambig_report.resumed.is_empty());
    assert_eq!(ambig_report.errors.len(), 2);
    assert!(!outside_wt.exists());

    let ambig_err = session::session_resume(&core, &shell.id, None, None)
        .unwrap_err()
        .to_string();
    assert!(
        ambig_err.contains("restore in main checkout"),
        "{ambig_err}"
    );

    // 4. External worktree started in tracked subfolder `../feat-outside/src`:
    //    restores at `../feat-outside` while keeping `session.cwd == ../feat-outside/src`,
    //    and refuses if a competing branch `"src"` also exists.
    git(&main_repo, &["branch", "-D", "feat-outside"]);
    let sub_src = outside_wt.join("src");
    git(
        &main_repo,
        &["worktree", "add", "--", &outside_str, "feat/outside"],
    );
    std::fs::create_dir_all(&sub_src).unwrap();
    std::fs::write(sub_src.join("mod.rs"), "pub fn m() {}\n").unwrap();
    git(&outside_wt, &["add", "-A"]);
    git(&outside_wt, &["commit", "-q", "-m", "add tracked src"]);

    let sub_chat = session::session_start_chat(
        &core,
        StartDirectSessionArgs {
            role_id: Some(role_id.clone()),
            cwd: Some(sub_src.to_string_lossy().into_owned()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(sub_chat.project_id.as_deref(), Some(project.id.as_str()));
    session::session_stop(&core, &sub_chat.session.id).unwrap();
    {
        let conn = core.db.get().unwrap();
        conn.execute(
            "UPDATE sessions SET status = 'stopped' WHERE id = ?1",
            [&sub_chat.session.id],
        )
        .unwrap();
    }

    git(&main_repo, &["worktree", "remove", &outside_str]);
    assert!(!outside_wt.exists());
    session::session_resume(&core, &sub_chat.session.id, None, None).unwrap();
    assert!(sub_src.is_dir());
    assert_eq!(
        repo::session::get_row(&core.db.get().unwrap(), &sub_chat.session.id)
            .unwrap()
            .unwrap()
            .cwd
            .as_deref(),
        Some(sub_src.to_string_lossy().as_ref())
    );

    session::session_stop(&core, &sub_chat.session.id).unwrap();
    {
        let conn = core.db.get().unwrap();
        conn.execute(
            "UPDATE sessions SET status = 'stopped' WHERE id = ?1",
            [&sub_chat.session.id],
        )
        .unwrap();
    }
    git(&main_repo, &["branch", "src", "main"]);
    git(&main_repo, &["worktree", "remove", &outside_str]);
    let sub_ambig_err = session::session_resume(&core, &sub_chat.session.id, None, None)
        .unwrap_err()
        .to_string();
    assert!(
        sub_ambig_err.contains("restore in main checkout"),
        "{sub_ambig_err}"
    );
    assert!(!outside_wt.exists());
    assert!(!sub_src.exists());

    // 5. Project-associated missing external non-Git plain folder retains Plain checkout,
    //    unarchive succeeds without Git error, and shell resume falls back to project cwd.
    let plain_dir = root.path().join("plain-external-folder");
    std::fs::create_dir_all(&plain_dir).unwrap();
    let plain_str = plain_dir.to_string_lossy().into_owned();
    let plain_chat = session::session_start_chat(
        &core,
        StartDirectSessionArgs {
            role_id: Some(role_id),
            project_id: Some(project.id.clone()),
            cwd: Some(plain_str.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    session::session_archive_direct(&core, &plain_chat.session.id).unwrap();

    let plain_shell = session::session_start_shell_in(
        &core,
        ProjectScope::Project(project.id.clone()),
        Some(plain_str),
        None,
        None,
        None,
    )
    .unwrap();
    {
        let mut conn = core.db.get().unwrap();
        conn.execute(
            "UPDATE sessions SET status = 'running' WHERE id = ?1",
            [&plain_shell.id],
        )
        .unwrap();
        repo::session::mark_running_for_resume_on_launch(&mut conn).unwrap();
    }
    session::session_stop(&core, &plain_shell.id).unwrap();
    {
        let conn = core.db.get().unwrap();
        conn.execute(
            "UPDATE sessions SET status = 'stopped' WHERE id = ?1",
            [&plain_shell.id],
        )
        .unwrap();
    }
    std::fs::remove_dir_all(&plain_dir).unwrap();
    assert_eq!(
        crate::files::worktree_checkout(&core, &plain_dir, Some(&project.id)).unwrap(),
        runner_core::protocol::worktree::WorktreeCheckout::Plain { missing: true }
    );
    session::session_unarchive(&core, &plain_chat.session.id).unwrap();
    assert!(!plain_dir.exists());

    let plain_report = crate::daemon::resume::consume_resume_on_launch_until(
        &core,
        true,
        |_| Some((80, 24)),
        &crate::daemon::resume::ResumeConsumer::default(),
    )
    .unwrap();
    assert_eq!(plain_report.resumed, vec![plain_shell.id.clone()]);
    assert!(plain_report.errors.is_empty(), "{:?}", plain_report.errors);
    assert!(!plain_dir.exists());
}
