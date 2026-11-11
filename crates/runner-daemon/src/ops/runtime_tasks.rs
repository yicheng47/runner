use std::sync::Arc;
use std::time::{Duration, Instant};

use runner_core::protocol::{Runtime, RuntimeUpdateState, RuntimeUpdateTask};

use crate::error::{Error, Result};
use crate::AppCore;

#[derive(Default)]
pub(crate) struct UpdateTasks {
    pub(crate) tasks: Vec<RuntimeUpdateTask>,
    all_finished: Option<Instant>,
    #[cfg(test)]
    monitor_barrier: Option<Arc<std::sync::Barrier>>,
}

impl UpdateTasks {
    fn changed(&mut self) {
        self.all_finished = if !self.tasks.is_empty()
            && self
                .tasks
                .iter()
                .all(|task| task.state == RuntimeUpdateState::Succeeded)
        {
            self.all_finished.or_else(|| Some(Instant::now()))
        } else {
            None
        };
    }

    fn clear_due(&self, now: Instant) -> bool {
        self.all_finished
            .is_some_and(|at| now.saturating_duration_since(at) >= Duration::from_secs(5))
    }
}

pub fn list(core: &AppCore) -> Vec<RuntimeUpdateTask> {
    core.sessions
        .runtime_updates
        .lock()
        .unwrap()
        .tasks
        .iter()
        .filter(|task| task.state != RuntimeUpdateState::Stopping)
        .cloned()
        .collect()
}

fn publish(core: &AppCore) {
    core.events.emit("runtime/task-changed", &list(core));
}

pub fn start(core: &AppCore, runtime: Runtime) -> Result<RuntimeUpdateTask> {
    let status = super::runtime::runtime_status_list(core)?
        .runtimes
        .into_iter()
        .find(|entry| entry.name == runtime)
        .ok_or_else(|| Error::msg("unknown runtime"))?;
    let mut updates = core.sessions.runtime_updates.lock().unwrap();
    if let Some(task) = updates
        .tasks
        .iter()
        .find(|task| task.runtime == runtime && task.state.active())
    {
        return Ok(task.clone());
    }
    let spec = super::runtime::runtime_update_spawn_spec(core, runtime, (80, 24))?;
    let id = spec.session_id.clone();
    let events: Arc<dyn crate::session::manager::SessionEvents> = Arc::new(core.session_events());
    core.sessions
        .prepare_unlisted_terminal(&id, (80, 24), &core.db, &events)?;
    let old_ids: Vec<_> = updates
        .tasks
        .iter()
        .filter(|task| task.runtime == runtime)
        .map(|task| task.id.clone())
        .collect();
    updates.tasks.retain(|task| task.runtime != runtime);
    for id in old_ids {
        core.sessions.discard_unlisted_terminal(&id);
    }
    let task = RuntimeUpdateTask {
        id: id.clone(),
        runtime,
        from_version: status.installed_version,
        to_version: status.available_version,
        state: RuntimeUpdateState::Running,
        step: "Starting update…".into(),
        message: None,
        exit_code: None,
        installed_version: None,
        running_sessions: 0,
    };
    updates.tasks.push(task.clone());
    updates.changed();
    #[cfg(test)]
    let monitor_barrier = updates.monitor_barrier.clone();
    let spawned = super::runtime::runtime_update_start(core, spec, events);
    drop(updates);
    publish(core);
    let completed = match spawned {
        Ok(completed) => completed,
        Err(error) => {
            finish(core, &id, None, Some(error.to_string()));
            return Ok(list(core)
                .into_iter()
                .find(|task| task.id == id)
                .unwrap_or(task));
        }
    };
    let core = core.clone();
    std::thread::spawn(move || {
        #[cfg(test)]
        if let Some(barrier) = monitor_barrier {
            barrier.wait();
        }
        let mut last_sequence = 0;
        let mut last_output = Instant::now();
        loop {
            match completed.try_recv() {
                Ok(code) => {
                    finish(&core, &id, code, None);
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    finish(
                        &core,
                        &id,
                        None,
                        Some("The update process ended without a result.".into()),
                    );
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
            if let Ok(model) = core.sessions.terminal_model(&id) {
                let sequence = core.sessions.terminal_output_sequence(&id);
                if sequence != last_sequence {
                    last_sequence = sequence;
                    last_output = Instant::now();
                }
                let rows = model.input_rows();
                let column = model.term.lock().grid().cursor.point.column.0;
                let step = rows
                    .iter()
                    .rev()
                    .find(|row| !row.trim().is_empty())
                    .map(|row| row.trim().to_owned())
                    .unwrap_or_default();
                let waiting = may_need_input(last_output.elapsed(), column);
                let mut updates = core.sessions.runtime_updates.lock().unwrap();
                let Some(task) = updates.tasks.iter_mut().find(|task| task.id == id) else {
                    return;
                };
                if task.state != RuntimeUpdateState::Stopping {
                    let state = if waiting {
                        RuntimeUpdateState::MayNeedInput
                    } else {
                        RuntimeUpdateState::Running
                    };
                    let changed = task.state != state || task.step != step;
                    task.state = state;
                    task.step = step;
                    drop(updates);
                    if changed {
                        publish(&core);
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    });
    Ok(task)
}

pub fn stop(core: &AppCore, id: &str) -> Result<()> {
    {
        let mut updates = core.sessions.runtime_updates.lock().unwrap();
        let task = updates
            .tasks
            .iter_mut()
            .find(|task| task.id == id)
            .ok_or_else(|| Error::msg("update task not found"))?;
        if !matches!(
            task.state,
            RuntimeUpdateState::Running | RuntimeUpdateState::MayNeedInput
        ) {
            return Ok(());
        }
        task.state = RuntimeUpdateState::Stopping;
        updates.changed();
    }
    publish(core);
    if let Err(error) = core.sessions.kill(id) {
        let mut updates = core.sessions.runtime_updates.lock().unwrap();
        if let Some(task) = updates.tasks.iter_mut().find(|task| task.id == id) {
            task.state = RuntimeUpdateState::Running;
            task.step = format!("Could not stop update: {error}");
        }
        drop(updates);
        publish(core);
        return Err(error);
    }
    Ok(())
}

pub fn dismiss(core: &AppCore, id: &str) -> Result<()> {
    let mut updates = core.sessions.runtime_updates.lock().unwrap();
    let Some(index) = updates.tasks.iter().position(|task| task.id == id) else {
        return Ok(());
    };
    if updates.tasks[index].state.active() {
        return Err(Error::msg("stop a running update before dismissing it"));
    }
    updates.tasks.remove(index);
    updates.changed();
    drop(updates);
    core.sessions.discard_unlisted_terminal(id);
    publish(core);
    schedule_clear(core);
    Ok(())
}

fn finish(core: &AppCore, id: &str, code: Option<i32>, error: Option<String>) {
    let runtime = {
        let mut updates = core.sessions.runtime_updates.lock().unwrap();
        let Some(task) = updates.tasks.iter_mut().find(|task| task.id == id) else {
            return;
        };
        if task.state != RuntimeUpdateState::Stopping {
            task.state = RuntimeUpdateState::Verifying;
        }
        task.runtime
    };
    publish(core);
    let final_line = core.sessions.terminal_model(id).ok().and_then(|model| {
        model
            .input_rows()
            .into_iter()
            .rev()
            .find(|row| !row.trim().is_empty())
    });
    let installed = super::runtime::runtime_probe_version(core, runtime);
    let works = installed.is_some()
        || crate::runtime_status::versions::starts_now(
            runtime,
            &core.db,
            &core.runtime_shell_env,
            &core.runtime_discovery,
        );
    let running = super::session::live_session_counts(core)
        .ok()
        .and_then(|counts| counts.get(&runtime).copied())
        .unwrap_or(0);
    let mut updates = core.sessions.runtime_updates.lock().unwrap();
    let Some(task) = updates.tasks.iter_mut().find(|task| task.id == id) else {
        return;
    };
    if let Some(line) = final_line {
        task.step = line.trim().to_owned();
    }
    let stopping = task.state == RuntimeUpdateState::Stopping;
    if stopping && works {
        updates.tasks.retain(|task| task.id != id);
        core.sessions.discard_unlisted_terminal(id);
    } else {
        task.state = if stopping {
            RuntimeUpdateState::BrokenAfterStop
        } else if code == Some(0) && error.is_none() {
            RuntimeUpdateState::Succeeded
        } else {
            RuntimeUpdateState::Failed
        };
        task.installed_version = installed;
        task.running_sessions = running;
        task.exit_code = code;
        task.message = error.or_else(|| match task.state {
            RuntimeUpdateState::Failed => Some(format!("{} {}", code.map_or_else(|| "The update ended without an exit code.".into(), |code| format!("Exited with code {code}.")), task.step)),
            RuntimeUpdateState::BrokenAfterStop => Some(format!("{} doesn't start after the update was stopped. Reinstall puts the latest version in place.", runtime.display_name())),
            _ => None,
        });
    }
    updates.changed();
    drop(updates);
    publish(core);
    schedule_clear(core);
}

fn schedule_clear(core: &AppCore) {
    if core
        .sessions
        .runtime_updates
        .lock()
        .unwrap()
        .all_finished
        .is_none()
    {
        return;
    }
    let core = core.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        let mut updates = core.sessions.runtime_updates.lock().unwrap();
        if !updates.clear_due(Instant::now()) {
            return;
        }
        let ids: Vec<_> = updates.tasks.drain(..).map(|task| task.id).collect();
        updates.all_finished = None;
        drop(updates);
        for id in ids {
            core.sessions.discard_unlisted_terminal(&id);
        }
        publish(&core);
    });
}

fn may_need_input(quiet: Duration, cursor_column: usize) -> bool {
    quiet >= Duration::from_secs(2) && cursor_column > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(state: RuntimeUpdateState) -> RuntimeUpdateTask {
        RuntimeUpdateTask {
            id: "update".into(),
            runtime: Runtime::Codex,
            from_version: Some("0.166.0".into()),
            to_version: Some("0.167.0".into()),
            state,
            step: String::new(),
            message: None,
            exit_code: None,
            installed_version: None,
            running_sessions: 0,
        }
    }

    #[test]
    fn clearing_waits_for_every_task_to_succeed_and_the_full_five_seconds() {
        let now = Instant::now();
        let mut updates = UpdateTasks {
            tasks: vec![task(RuntimeUpdateState::Succeeded)],
            all_finished: None,
            ..Default::default()
        };
        updates.changed();
        let at = updates.all_finished.unwrap();
        assert!(!updates.clear_due(at + Duration::from_millis(4999)));
        assert!(updates.clear_due(at + Duration::from_secs(5)));
        for state in [
            RuntimeUpdateState::Running,
            RuntimeUpdateState::MayNeedInput,
            RuntimeUpdateState::Failed,
            RuntimeUpdateState::BrokenAfterStop,
        ] {
            updates.tasks.push(task(state));
            updates.changed();
            assert!(!updates.clear_due(now + Duration::from_secs(60)));
            updates.tasks.pop();
        }
        updates.changed();
        assert!(updates.all_finished.unwrap() >= at);
    }

    #[test]
    fn input_guess_requires_two_seconds_and_a_midline_cursor() {
        assert!(!may_need_input(Duration::from_millis(1999), 10));
        assert!(!may_need_input(Duration::from_secs(2), 0));
        assert!(may_need_input(Duration::from_secs(2), 10));
    }

    #[cfg(unix)]
    fn fixture(script: &str) -> (tempfile::TempDir, AppCore) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("codex");
        std::fs::write(&executable, script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut core = crate::test_support::test_core_in(dir.path().into());
        core.sessions = crate::session::SessionManager::new(
            core.runtime_shell_env.clone(),
            core.runtime_discovery.clone(),
            Arc::new(crate::session::pty_runtime::PtyRuntime::new()),
        );
        crate::db::set_runtime_override(&core.db, "codex", Some(executable.to_str().unwrap()))
            .unwrap();
        (dir, core)
    }

    #[cfg(unix)]
    fn await_task(core: &AppCore, expected: RuntimeUpdateState) -> RuntimeUpdateTask {
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(task) = list(core).into_iter().find(|task| task.state == expected) {
                return task;
            }
            assert!(
                Instant::now() < until,
                "expected {expected:?}, got {:?}",
                list(core)
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(unix)]
    #[test]
    fn duplicate_join_input_success_and_finished_output_are_daemon_owned() {
        let (_dir, core) = fixture("#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'codex-cli 0.167.0'; exit; fi\nprintf 'Password: '\nread answer\necho installed\n");
        let first = start(&core, Runtime::Codex).unwrap();
        let joined = start(&core, Runtime::Codex).unwrap();
        assert_eq!(first.id, joined.id);
        assert_eq!(list(&core).len(), 1);
        await_task(&core, RuntimeUpdateState::MayNeedInput);
        core.sessions.desktop_input(&first.id, b"answer\r").unwrap();
        let result = await_task(&core, RuntimeUpdateState::Succeeded);
        assert_eq!(result.installed_version.as_deref(), Some("0.167.0"));
        assert!(
            core.sessions.attach_terminal(&first.id).is_ok(),
            "finished tasks retain their output terminal"
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        while !list(&core).is_empty() {
            assert!(Instant::now() < deadline, "successful tasks did not clear");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(core.sessions.terminal_model(&first.id).is_err());
        core.sessions.drain_terminal_workers();
    }

    #[cfg(unix)]
    #[test]
    fn failure_stays_until_retry_or_dismissal() {
        let (_dir, core) = fixture("#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'codex-cli 0.166.0'; exit; fi\necho 'permission denied'\nexit 243\n");
        let first = start(&core, Runtime::Codex).unwrap();
        let failed = await_task(&core, RuntimeUpdateState::Failed);
        assert_eq!(failed.exit_code, Some(243));
        assert!(failed.message.as_deref().unwrap().contains("243"));
        assert!(core.sessions.attach_terminal(&first.id).is_ok());
        let retry = start(&core, Runtime::Codex).unwrap();
        assert_ne!(retry.id, first.id);
        assert!(core.sessions.terminal_model(&first.id).is_err());
        await_task(&core, RuntimeUpdateState::Failed);
        dismiss(&core, &retry.id).unwrap();
        core.sessions.drain_terminal_workers();
    }

    #[cfg(unix)]
    #[test]
    fn stopping_reprobes_and_retains_only_a_broken_executable() {
        for broken in [false, true] {
            let (dir, core) = fixture("#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'codex-cli 0.166.0'; exit; fi\nprintf 'Installing…'\nsleep 60\n");
            let task = start(&core, Runtime::Codex).unwrap();
            if broken {
                std::fs::write(dir.path().join("codex"), "#!/bin/sh\nexit 1\n").unwrap();
            }
            stop(&core, &task.id).unwrap();
            if broken {
                await_task(&core, RuntimeUpdateState::BrokenAfterStop);
                dismiss(&core, &task.id).unwrap();
            } else {
                let until = Instant::now() + Duration::from_secs(10);
                while core
                    .sessions
                    .runtime_updates
                    .lock()
                    .unwrap()
                    .tasks
                    .iter()
                    .any(|entry| entry.id == task.id)
                {
                    assert!(Instant::now() < until);
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            core.sessions.drain_terminal_workers();
        }
    }
    #[cfg(unix)]
    #[test]
    fn dismissing_a_non_task_cannot_discard_a_live_chat() {
        let (dir, core) = fixture("#!/bin/sh\necho 'codex-cli 0.167.0'\n");
        let mut role =
            crate::session::manager::runtime_direct_role("shell", Some("/bin/sh"), None, None)
                .unwrap();
        role.args = vec!["-c".into(), "read -r line".into()];
        let session = core
            .sessions
            .spawn_runtime_direct(
                &role,
                None,
                dir.path().to_str(),
                None,
                None,
                dir.path(),
                core.db.clone(),
                Arc::new(core.session_events()),
            )
            .unwrap();
        assert!(core.sessions.session_live(&session.id));
        assert!(core.sessions.terminal_model(&session.id).is_ok());
        let client = crate::daemon::InProcessTransport::client(core.clone());
        client.runtime_task_dismiss(&session.id).unwrap();
        assert!(core.sessions.session_live(&session.id));
        assert!(core.sessions.attach_terminal(&session.id).is_ok());
        core.sessions.kill(&session.id).unwrap();
        core.sessions.drain_terminal_workers();
    }

    #[cfg(unix)]
    #[test]
    fn completion_survives_forced_broadcast_lag_and_allows_retry_and_clear() {
        let (_dir, core) =
            fixture("#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'codex-cli 0.167.0'; fi\n");
        let barrier = Arc::new(std::sync::Barrier::new(2));
        core.sessions
            .runtime_updates
            .lock()
            .unwrap()
            .monitor_barrier = Some(barrier.clone());
        let mut events = core.events.subscribe();
        let task = start(&core, Runtime::Codex).unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Ok(event) = events.try_recv() {
                if event.name == "session/exit" && event.payload["session_id"] == task.id {
                    break;
                }
            }
            assert!(Instant::now() < deadline, "update process never exited");
            std::thread::sleep(Duration::from_millis(10));
        }
        for index in 0..20_000 {
            core.events.emit("test/flood", &index);
        }
        assert!(matches!(
            events.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_))
        ));
        core.sessions
            .runtime_updates
            .lock()
            .unwrap()
            .monitor_barrier = None;
        barrier.wait();
        await_task(&core, RuntimeUpdateState::Succeeded);
        let retry = start(&core, Runtime::Codex).unwrap();
        assert_ne!(retry.id, task.id);
        await_task(&core, RuntimeUpdateState::Succeeded);
        let deadline = Instant::now() + Duration::from_secs(15);
        while !list(&core).is_empty() {
            assert!(
                Instant::now() < deadline,
                "completed update did not clear after retry"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(core.sessions.terminal_model(&retry.id).is_err());
        core.sessions.drain_terminal_workers();
    }
}
