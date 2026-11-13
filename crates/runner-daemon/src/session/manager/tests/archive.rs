use super::*;
use crate::ops::session::{
    session_archive_direct, session_close, session_resume, session_stop, session_unarchive,
};
use runner_core::protocol::terminal::{TerminalAttachment, TerminalFrame};

fn core_with_fake(data: &Path, fake: Arc<FakeRuntime>) -> crate::AppCore {
    let mut core = crate::test_support::test_core_in(data.to_owned());
    core.db = Arc::new(db::open_pool(&data.join("runner.db")).unwrap());
    core.sessions = manager_with_runtime(
        crate::shell_path::LoginShellEnv::default(),
        fake as Arc<dyn SessionRuntime>,
    );
    core
}

fn spawn(core: &crate::AppCore, runtime: &str) -> String {
    let configured = runtime_direct_role(runtime, Some("fake-agent"), None, None).unwrap();
    core.sessions
        .spawn_runtime_direct(
            &configured,
            None,
            Some(fixture_tmp_dir().to_str().unwrap()),
            Some(80),
            Some(24),
            &core.app_data_dir,
            core.db.clone(),
            Arc::new(core.session_events()),
        )
        .unwrap()
        .id
}

fn output(
    core: &crate::AppCore,
    fake: &FakeRuntime,
    id: &str,
    attachment: &mut TerminalAttachment,
    seq: u64,
    bytes: &[u8],
) {
    fake.push_output(fake.spawn_count() - 1, bytes);
    assert!(matches!(
        attachment.frames.recv().unwrap(),
        TerminalFrame::Output { seq: actual, bytes: actual_bytes }
            if actual == seq && actual_bytes == bytes
    ));
    // Taking the state lock waits for the same chunk's parser call to finish.
    core.sessions.terminal_model(id).unwrap().flush_events();
    assert_eq!(core.sessions.terminal_output_sequence(id), seq);
}

fn color_probe(
    core: &crate::AppCore,
    fake: &FakeRuntime,
    id: &str,
    attachment: &mut TerminalAttachment,
    seq: u64,
) {
    fake.inputs.lock().unwrap().clear();
    output(
        core,
        fake,
        id,
        attachment,
        seq,
        b"FIRST-OUTPUT\x1b]10;?\x1b\\\x1b]11;?\x1b\\",
    );
    let writes: Vec<_> = fake
        .bytes_writes()
        .into_iter()
        .filter(|(_, bytes)| !bytes.is_empty())
        .collect();
    assert_eq!(
        writes.len(),
        2,
        "both startup color queries must be answered"
    );
    assert!(writes[0].1.starts_with(b"\x1b]10;rgb:"));
    assert!(writes[1].1.starts_with(b"\x1b]11;rgb:"));
    assert!(core
        .sessions
        .terminal_model(id)
        .unwrap()
        .input_rows()
        .iter()
        .any(|row| row.contains("FIRST-OUTPUT")));
}

#[test]
fn archive_restore_resumes_first_chunk_with_fresh_terminal_and_color_replies() {
    use alacritty_terminal::term::TermMode;

    let data = tempfile::tempdir().unwrap();
    let fake = fake_runtime();
    let core = core_with_fake(data.path(), fake.clone());
    let id = spawn(&core, "trae");
    let key = uuid::Uuid::new_v4().to_string();
    core.db
        .get()
        .unwrap()
        .execute(
            "UPDATE sessions SET agent_session_key = ?2 WHERE id = ?1",
            params![id, key],
        )
        .unwrap();
    let definition = crate::repo::session::get_row(&core.db.get().unwrap(), &id)
        .unwrap()
        .unwrap()
        .launch_definition;
    let mut attachment = core.sessions.attach_terminal(&id).unwrap();
    color_probe(&core, &fake, &id, &mut attachment, 1);

    for _ in 0..2 {
        let old = core.sessions.terminal_model(&id).unwrap();
        for _ in 0..20 {
            let seq = core.sessions.terminal_output_sequence(&id) + 1;
            output(
                &core,
                &fake,
                &id,
                &mut attachment,
                seq,
                b"\x1b[?1049h\x1b[?2004hOLD-HISTORY\r\n",
            );
        }
        let (mut companion_attachment, companion) =
            core.sessions.attach_companion_terminal(&id, 10).unwrap();
        let state = core.sessions.session_state(&id).unwrap();
        assert!(state.lock().unwrap().terminal_input_worker.is_some());

        let old_subscriber = attachment.subscriber_id;
        session_archive_direct(&core, &id).unwrap();
        assert!(core.sessions.terminal_model(&id).is_err());
        assert_eq!(core.sessions.terminal_output_sequence(&id), 0);
        {
            let state = state.lock().unwrap();
            assert!(state.terminal.is_none());
            assert!(state.terminal_input.is_none());
            assert!(state.terminal_input_worker.is_none());
            assert!(state.subscribers.is_empty());
            assert!(state.companion_terminals.is_empty());
        }
        assert!(attachment.frames.recv().is_err());
        assert!(companion_attachment.frames.recv().is_err());
        let mut final_attachment = companion.attach().unwrap();
        assert!(final_attachment.frames.recv().is_err());
        assert!(final_attachment
            .snapshot
            .bytes
            .windows(b"OLD-HISTORY".len())
            .any(|bytes| bytes == b"OLD-HISTORY"));

        session_unarchive(&core, &id).unwrap();
        assert_eq!(session_resume(&core, &id, None, None).unwrap().id, id);
        let fresh = core.sessions.terminal_model(&id).unwrap();
        assert!(!Arc::ptr_eq(&old, &fresh));
        assert!(!fresh
            .term
            .lock()
            .mode()
            .intersects(TermMode::ALT_SCREEN | TermMode::BRACKETED_PASTE));
        attachment = core.sessions.attach_terminal(&id).unwrap();
        assert_eq!(attachment.snapshot.seq, 0);
        color_probe(&core, &fake, &id, &mut attachment, 1);
        core.sessions.detach_terminal(&id, old_subscriber);
        color_probe(&core, &fake, &id, &mut attachment, 2);

        assert!(old.write_input(b"stale input").is_err());
        assert!(old.feed_output(1000, b"\x1b]10;?\x1b\\").is_none());
        assert_eq!(
            fake.bytes_writes()
                .iter()
                .filter(|(_, bytes)| !bytes.is_empty())
                .count(),
            2
        );
        let row = crate::repo::session::get_row(&core.db.get().unwrap(), &id)
            .unwrap()
            .unwrap();
        assert_eq!(row.agent_session_key.as_deref(), Some(key.as_str()));
        assert_eq!(row.launch_definition, definition);
        assert!(row.archived_at.is_none());
    }
    session_stop(&core, &id).unwrap();
    core.sessions.drain_terminal_workers();
}

#[test]
fn ordinary_stop_resume_keeps_sequence_terminal_and_single_input_worker() {
    let data = tempfile::tempdir().unwrap();
    let fake = fake_runtime();
    let core = core_with_fake(data.path(), fake.clone());
    let id = spawn(&core, "trae");
    let old = core.sessions.terminal_model(&id).unwrap();
    let state = core.sessions.session_state(&id).unwrap();
    let worker = state
        .lock()
        .unwrap()
        .terminal_input_worker
        .as_ref()
        .unwrap()
        .thread()
        .id();
    let mut attachment = core.sessions.attach_terminal(&id).unwrap();
    for seq in 1..=20 {
        output(&core, &fake, &id, &mut attachment, seq, b"history\r\n");
    }
    session_stop(&core, &id).unwrap();
    session_resume(&core, &id, None, None).unwrap();
    assert!(Arc::ptr_eq(
        &old,
        &core.sessions.terminal_model(&id).unwrap()
    ));
    assert_eq!(
        state
            .lock()
            .unwrap()
            .terminal_input_worker
            .as_ref()
            .unwrap()
            .thread()
            .id(),
        worker
    );
    color_probe(&core, &fake, &id, &mut attachment, 21);
    // A forget racing a live attachment must not reset its parser or sequence.
    core.sessions.forget_session_state(&id);
    color_probe(&core, &fake, &id, &mut attachment, 22);
    session_stop(&core, &id).unwrap();
    core.sessions.drain_terminal_workers();
}

#[test]
fn stop_resume_rebinds_reused_terminal_for_live_title_updates() {
    let data = tempfile::tempdir().unwrap();
    let fake = fake_runtime();
    let core = core_with_fake(data.path(), fake.clone());

    // Resumed conversation keeps the terminal model and accepts new OSC 2 titles
    // after `resume_in_place` advances `started_at`.
    let id = spawn(&core, "trae");
    let key = uuid::Uuid::new_v4().to_string();
    core.db
        .get()
        .unwrap()
        .execute(
            "UPDATE sessions SET agent_session_key = ?2 WHERE id = ?1",
            params![id, key],
        )
        .unwrap();
    let old = core.sessions.terminal_model(&id).unwrap();
    let mut attachment = core.sessions.attach_terminal(&id).unwrap();
    output(
        &core,
        &fake,
        &id,
        &mut attachment,
        1,
        b"\x1b]2;Provider First\x07",
    );
    assert_eq!(
        crate::repo::session::get_row(&core.db.get().unwrap(), &id)
            .unwrap()
            .unwrap()
            .live_title
            .as_deref(),
        Some("Provider First")
    );
    std::thread::sleep(Duration::from_millis(5));
    session_stop(&core, &id).unwrap();
    session_resume(&core, &id, None, None).unwrap();
    assert!(Arc::ptr_eq(
        &old,
        &core.sessions.terminal_model(&id).unwrap()
    ));
    assert_eq!(
        crate::repo::session::get_row(&core.db.get().unwrap(), &id)
            .unwrap()
            .unwrap()
            .live_title
            .as_deref(),
        Some("Provider First")
    );
    output(
        &core,
        &fake,
        &id,
        &mut attachment,
        2,
        b"\x1b]2;Resumed Provider\x07",
    );
    assert_eq!(
        crate::repo::session::get_row(&core.db.get().unwrap(), &id)
            .unwrap()
            .unwrap()
            .live_title
            .as_deref(),
        Some("Resumed Provider")
    );
    session_stop(&core, &id).unwrap();

    // Fresh-fallback resume clears `live_title`; re-emitting the same OSC 2 title
    // on the reused terminal must still persist instead of hitting stale dedupe.
    let fresh_id = spawn(&core, "trae");
    let fresh_old = core.sessions.terminal_model(&fresh_id).unwrap();
    let mut fresh_attachment = core.sessions.attach_terminal(&fresh_id).unwrap();
    output(
        &core,
        &fake,
        &fresh_id,
        &mut fresh_attachment,
        1,
        b"\x1b]2;Same Topic\x07",
    );
    assert_eq!(
        crate::repo::session::get_row(&core.db.get().unwrap(), &fresh_id)
            .unwrap()
            .unwrap()
            .live_title
            .as_deref(),
        Some("Same Topic")
    );
    std::thread::sleep(Duration::from_millis(5));
    session_stop(&core, &fresh_id).unwrap();
    // Simulate a late pre-rebind OSC 2 event sitting in the terminal queue after
    // `resume_in_place` already advanced `started_at` and cleared `live_title`:
    // it must execute under the old generation (matching 0 rows) before rebind.
    let stale_started_at = core
        .sessions
        .session_state(&fresh_id)
        .unwrap()
        .lock()
        .unwrap()
        .terminal_started_at
        .clone()
        .unwrap();
    let resumed_at = Utc::now() + chrono::Duration::seconds(2);
    crate::repo::session::resume_in_place(
        &core.db.get().unwrap(),
        &fresh_id,
        resumed_at,
        None,
        true,
        (80, 24),
        None,
    )
    .unwrap();
    core.sessions.ingest_output_chunk(
        &fresh_id,
        None,
        b"\x1b]2;Stale Pre-Resume\x07",
        &core.session_events(),
    );
    assert!(matches!(
        fresh_attachment.frames.recv().unwrap(),
        TerminalFrame::Output { seq: 2, .. }
    ));
    core.sessions
        .rebind_terminal_session(&fresh_id, &fresh_old, &stale_started_at, &core.db)
        .unwrap();
    fresh_old.flush_events();
    assert_eq!(
        crate::repo::session::get_row(&core.db.get().unwrap(), &fresh_id)
            .unwrap()
            .unwrap()
            .live_title,
        None
    );
    crate::repo::session::set_exit_status(
        &core.db.get().unwrap(),
        &fresh_id,
        crate::model::SessionStatus::Stopped,
        resumed_at,
    )
    .unwrap();
    session_resume(&core, &fresh_id, None, None).unwrap();
    assert!(Arc::ptr_eq(
        &fresh_old,
        &core.sessions.terminal_model(&fresh_id).unwrap()
    ));
    assert_eq!(
        crate::repo::session::get_row(&core.db.get().unwrap(), &fresh_id)
            .unwrap()
            .unwrap()
            .live_title,
        None
    );
    output(
        &core,
        &fake,
        &fresh_id,
        &mut fresh_attachment,
        3,
        b"\x1b]2;Same Topic\x07",
    );
    assert_eq!(
        crate::repo::session::get_row(&core.db.get().unwrap(), &fresh_id)
            .unwrap()
            .unwrap()
            .live_title
            .as_deref(),
        Some("Same Topic")
    );
    session_stop(&core, &fresh_id).unwrap();
    core.sessions.drain_terminal_workers();
}

#[test]
fn explicit_and_natural_shell_close_release_terminal_resources() {
    for natural in [false, true] {
        let data = tempfile::tempdir().unwrap();
        let fake = fake_runtime();
        let core = core_with_fake(data.path(), fake.clone());
        let id = spawn(&core, "shell");
        let model = Arc::downgrade(&core.sessions.terminal_model(&id).unwrap());
        let mut attachment = core.sessions.attach_terminal(&id).unwrap();
        output(&core, &fake, &id, &mut attachment, 1, b"history");
        if natural {
            let forwarder = core
                .sessions
                .session_state(&id)
                .unwrap()
                .lock()
                .unwrap()
                .handle
                .as_mut()
                .unwrap()
                .forwarder
                .take()
                .unwrap();
            fake.close_spawn(0);
            forwarder.join().unwrap();
        } else {
            session_close(&core, &id).unwrap();
        }
        assert!(model.upgrade().is_none());
        assert!(attachment.frames.recv().is_err());
        assert!(crate::repo::session::get_row(&core.db.get().unwrap(), &id)
            .unwrap()
            .is_none());
        let state = core.sessions.raw_session_state(&id);
        assert!(state.is_none_or(|state| {
            let state = state.lock().unwrap();
            state.terminal.is_none() && state.terminal_input_worker.is_none()
        }));
        core.sessions.drain_terminal_workers();
    }
}

#[test]
fn exit_does_not_hold_terminal_control_while_waiting_for_runtime_updates() {
    let data = tempfile::tempdir().unwrap();
    let core = core_with_fake(data.path(), fake_runtime());
    let manager = &core.sessions;
    install_test_session_handle(manager, "exited");
    let runtime_session = manager.live_runtime_session("exited").unwrap();
    let budget = ci_scaled_budget(Duration::from_secs(5));
    let (entered, entry) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    *manager.terminal_retention_read.lock().unwrap() = Some(Box::new(move || {
        entered.send(()).unwrap();
        resume.recv_timeout(budget).unwrap();
    }));
    let updates = manager.runtime_updates.lock().unwrap();
    let (finished, completion) = std::sync::mpsc::channel();
    let exiting = manager.clone();
    let worker = thread::spawn(move || {
        finished
            .send(exiting.forget_runtime_handle("exited", &runtime_session))
            .unwrap();
    });
    entry.recv_timeout(budget).unwrap();

    let control_available = manager.terminal_control.try_lock().is_ok();
    if control_available {
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        manager
            .prepare_unlisted_terminal("prepared-update", (80, 24), &core.db, &events)
            .unwrap();
    }
    release.send(()).unwrap();
    drop(updates);
    completion.recv_timeout(budget).unwrap().unwrap();
    worker.join().unwrap();
    manager.drain_terminal_workers();
    assert!(
        control_available,
        "an update holding runtime_updates must still be able to prepare its terminal"
    );
}

#[test]
fn forget_cannot_split_terminal_and_live_handle_installation() {
    struct InstallEvents(
        Arc<SessionManager>,
        Mutex<std::sync::mpsc::Receiver<()>>,
        Duration,
    );
    impl SessionEvents for InstallEvents {
        fn exit(&self, _: &ExitEvent) {}

        fn spawned(&self, event: &SessionSpawnedEvent) {
            self.1.lock().unwrap().recv_timeout(self.2).unwrap();
            assert!(self.0.terminal_control.try_lock().is_ok());
            let state = self.0.session_state(&event.session_id).unwrap();
            let state = state.lock().unwrap();
            assert!(state.handle.is_some());
            assert!(state.terminal.is_some());
        }
    }

    let data = tempfile::tempdir().unwrap();
    let fake = fake_runtime();
    let core = core_with_fake(data.path(), fake.clone());
    let id = "installing";
    let manager = &core.sessions;
    let state = manager.session_state_or_insert(id);
    let gate = state.lock().unwrap().delivery_gate.clone();
    let delivery = gate.state.lock().unwrap();
    let budget = ci_scaled_budget(Duration::from_secs(5));
    let (runtime_session, stream) = fake
        .spawn(SpawnSpec {
            session_id: id.into(),
            ..Default::default()
        })
        .unwrap();
    let (installed, installation) = std::sync::mpsc::channel();
    let (cleaned, cleanup_complete) = std::sync::mpsc::channel();
    let installing = manager.clone();
    let pool = core.db.clone();
    let worker = thread::spawn(move || {
        let events: Arc<dyn SessionEvents> = Arc::new(InstallEvents(
            installing.clone(),
            Mutex::new(cleanup_complete),
            budget,
        ));
        installing.install_handle(
            id,
            SessionHandle {
                startup: None,
                #[cfg(windows)]
                pending_first_turn: None,
                id: id.into(),
                mission_id: None,
                role_id: None,
                runtime_session,
                rollout_capture: None,
                forwarder: None,
                stop: stream.stop_flag(),
            },
            None,
            Some((80, 24)),
            &pool,
            &events,
        );
        installed.send(()).unwrap();
    });
    let deadline = Instant::now() + budget;
    while state.lock().unwrap().terminal.is_none() {
        assert!(Instant::now() < deadline, "terminal was never installed");
        thread::yield_now();
    }
    // The held delivery gate pauses installation between the model and handle.
    let cleanup_excluded = manager.terminal_control.try_lock().is_err();
    let (forgotten, cleanup) = std::sync::mpsc::channel();
    let forgetting = manager.clone();
    let cleanup_worker = thread::spawn(move || {
        forgetting.forget_session_state(id);
        cleaned.send(()).unwrap();
        forgotten.send(()).unwrap();
    });
    drop(delivery);
    installation.recv_timeout(budget).unwrap();
    cleanup.recv_timeout(budget).unwrap();
    worker.join().unwrap();
    cleanup_worker.join().unwrap();
    let intact = {
        let state = state.lock().unwrap();
        state.handle.is_some()
            && state.terminal.is_some()
            && state.terminal_input.is_some()
            && state.terminal_input_worker.is_some()
    };
    manager.kill(id).unwrap();
    manager.drain_terminal_workers();
    assert!(cleanup_excluded, "forget must wait for the new live handle");
    assert!(
        intact,
        "forget must preserve the newly installed live stream"
    );
}
