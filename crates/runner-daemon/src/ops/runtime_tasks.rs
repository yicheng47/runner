use std::sync::Arc;
use std::time::{Duration, Instant};

use runner_core::protocol::{Runtime, RuntimeTaskKind, RuntimeUpdateState, RuntimeUpdateTask};

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
    start_task(core, runtime, RuntimeTaskKind::Update)
}

pub fn start_install(core: &AppCore, runtime: Runtime) -> Result<RuntimeUpdateTask> {
    start_task(core, runtime, RuntimeTaskKind::Install)
}

fn start_task(
    core: &AppCore,
    runtime: Runtime,
    kind: RuntimeTaskKind,
) -> Result<RuntimeUpdateTask> {
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
    let spec = match kind {
        RuntimeTaskKind::Update => {
            super::runtime::runtime_update_spawn_spec(core, runtime, (80, 24))?
        }
        RuntimeTaskKind::Install => {
            super::runtime::runtime_install_spawn_spec(core, runtime, (80, 24))?
        }
    };
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
        kind,
        from_version: match kind {
            RuntimeTaskKind::Update => status.installed_version,
            RuntimeTaskKind::Install => None,
        },
        to_version: match kind {
            RuntimeTaskKind::Update => status.available_version,
            RuntimeTaskKind::Install => None,
        },
        state: RuntimeUpdateState::Running,
        step: match kind {
            RuntimeTaskKind::Update => "Starting update…".into(),
            RuntimeTaskKind::Install => "Starting install…".into(),
        },
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
    let (runtime, kind) = {
        let mut updates = core.sessions.runtime_updates.lock().unwrap();
        let Some(task) = updates.tasks.iter_mut().find(|task| task.id == id) else {
            return;
        };
        if task.state != RuntimeUpdateState::Stopping {
            task.state = RuntimeUpdateState::Verifying;
        }
        (task.runtime, task.kind)
    };
    publish(core);
    let final_line = core.sessions.terminal_model(id).ok().and_then(|model| {
        model
            .input_rows()
            .into_iter()
            .rev()
            .find(|row| !row.trim().is_empty())
    });
    let stopping_now = core
        .sessions
        .runtime_updates
        .lock()
        .unwrap()
        .tasks
        .iter()
        .find(|task| task.id == id)
        .is_some_and(|task| task.state == RuntimeUpdateState::Stopping);
    let (installed, works) = if stopping_now && kind == RuntimeTaskKind::Install {
        (None, true)
    } else {
        let installed = super::runtime::runtime_probe_version(core, runtime);
        let works = installed.is_some()
            || crate::runtime_status::versions::starts_now(
                runtime,
                &core.db,
                &core.runtime_shell_env,
                &core.runtime_discovery,
            );
        (installed, works)
    };
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
    let emit_runtime_changed = !stopping && code == Some(0) && error.is_none();
    if stopping && (works || kind == RuntimeTaskKind::Install) {
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
            RuntimeUpdateState::Failed => Some(format!(
                "{} {}",
                code.map_or_else(
                    || match kind {
                        RuntimeTaskKind::Update => "The update ended without an exit code.".into(),
                        RuntimeTaskKind::Install => {
                            "The install ended without an exit code.".into()
                        }
                    },
                    |code| format!("Exited with code {code}.")
                ),
                task.step
            )),
            RuntimeUpdateState::BrokenAfterStop => Some(format!(
                "{} doesn't start after the update was stopped. Reinstall puts the latest version in place.",
                runtime.display_name()
            )),
            _ => None,
        });
    }
    updates.changed();
    drop(updates);
    if emit_runtime_changed {
        core.events.emit("runtime/changed", &());
        crate::runtime_status::models::request(
            &core.db,
            &core.runtime_shell_env,
            &core.runtime_discovery,
            &core.events,
            &[runtime],
            true,
        );
        if runtime == Runtime::ClaudeCode {
            crate::runtimes::claude_code::accounts::refresh(core);
        }
    }
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
            kind: RuntimeTaskKind::Update,
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
        crate::test_support::isolate_runtime_path(&core, None);
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
            let (dir, core) = fixture("#!/bin/sh\nPATH=\"/usr/bin:/bin${PATH:+:$PATH}\"\nif [ \"$1\" = --version ]; then echo 'codex-cli 0.166.0'; exit; fi\nprintf 'Installing…'\nsleep 60\n");
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
            .spawn_direct(
                &role,
                crate::session::manager::DirectLaunch {
                    cwd: dir.path().to_str().map(str::to_owned),
                    runtime_direct: true,
                    ..Default::default()
                },
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

    #[cfg(unix)]
    fn install_fixture(npm_script: &str) -> (tempfile::TempDir, AppCore) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let npm = dir.path().join("npm");
        std::fs::write(&npm, npm_script).unwrap();
        std::fs::set_permissions(&npm, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut core = crate::test_support::test_core_in(dir.path().into());
        core.sessions = crate::session::SessionManager::new(
            core.runtime_shell_env.clone(),
            core.runtime_discovery.clone(),
            Arc::new(crate::session::pty_runtime::PtyRuntime::new()),
        );
        crate::test_support::isolate_runtime_path(&core, Some(dir.path()));
        (dir, core)
    }

    #[cfg(unix)]
    #[test]
    fn install_task_reprobes_and_emits_runtime_changed_on_success() {
        let dir = tempfile::tempdir().unwrap();
        let bin_dir = dir.path().display().to_string();
        let npm_script = format!(
            "#!/bin/sh\ncat <<'EOF' > {bin_dir}/pi\n#!/bin/sh\necho '9.9.9'\nEOF\nchmod 755 {bin_dir}/pi\necho 'installed @earendil-works/pi-coding-agent'\n"
        );
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(dir.path().join("npm"), npm_script).unwrap();
        std::fs::set_permissions(
            dir.path().join("npm"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();

        let mut core = crate::test_support::test_core_in(dir.path().into());
        core.sessions = crate::session::SessionManager::new(
            core.runtime_shell_env.clone(),
            core.runtime_discovery.clone(),
            Arc::new(crate::session::pty_runtime::PtyRuntime::new()),
        );
        crate::test_support::isolate_runtime_path(&core, Some(dir.path()));

        let mut events = core.events.subscribe();
        let started = start_install(&core, Runtime::Pi).unwrap();
        assert_eq!(started.kind, RuntimeTaskKind::Install);
        assert!(started.from_version.is_none());

        let finished = await_task(&core, RuntimeUpdateState::Succeeded);
        assert_eq!(finished.kind, RuntimeTaskKind::Install);
        assert_eq!(finished.installed_version.as_deref(), Some("9.9.9"));

        let mut saw_runtime_changed = false;
        while let Ok(evt) = events.try_recv() {
            if evt.name == "runtime/changed" {
                saw_runtime_changed = true;
            }
        }
        assert!(
            saw_runtime_changed,
            "successful install must emit runtime/changed"
        );

        let status = crate::ops::runtime::runtime_status_list(&core).unwrap();
        let pi = status
            .runtimes
            .iter()
            .find(|r| r.name == Runtime::Pi)
            .unwrap();
        assert_eq!(
            pi.state,
            runner_core::protocol::runtime::RuntimeRowState::Detected
        );
        assert_eq!(pi.installed_version.as_deref(), Some("9.9.9"));
        core.sessions.drain_terminal_workers();
    }

    #[cfg(unix)]
    #[test]
    fn install_task_failure_retries_and_stopped_install_clears_without_broken_state() {
        let (dir, core) = install_fixture("#!/bin/sh\necho 'permission denied'\nexit 1\n");
        let failed_start = start_install(&core, Runtime::Pi).unwrap();
        let failed = await_task(&core, RuntimeUpdateState::Failed);
        assert_eq!(failed.id, failed_start.id);
        assert_eq!(failed.kind, RuntimeTaskKind::Install);

        // Retry with a slow npm and stop it mid-flight: stopped installs must clear
        // instead of transitioning into BrokenAfterStop ("Reinstall").
        std::fs::write(dir.path().join("npm"), "#!/bin/sh\nsleep 30\n").unwrap();
        let retried = start_install(&core, Runtime::Pi).unwrap();
        assert_ne!(retried.id, failed.id);
        stop(&core, &retried.id).unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if list(&core).is_empty() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "stopped install task must clear without BrokenAfterStop, got {:?}",
                list(&core)
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        core.sessions.drain_terminal_workers();
    }

    #[test]
    fn install_task_rejects_missing_npm_or_non_npm_runtime() {
        let empty_dir = tempfile::tempdir().unwrap();
        let core = crate::test_support::test_core();
        crate::test_support::isolate_runtime_path(&core, Some(empty_dir.path()));

        assert!(start_install(&core, Runtime::Antigravity).is_err());
        let missing_npm = start_install(&core, Runtime::ClaudeCode).unwrap_err();
        assert!(missing_npm.to_string().contains("npm"), "{missing_npm}");

        // Even when npm is on PATH, Antigravity still rejects npm install -g.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let npm = empty_dir.path().join("npm");
            std::fs::write(&npm, "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(&npm, std::fs::Permissions::from_mode(0o755)).unwrap();
            let err = start_install(&core, Runtime::Antigravity).unwrap_err();
            assert!(err.to_string().contains("npm"), "{err}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn antigravity_update_task_runs_agy_update_without_disable_auto_update_and_reprobes() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let version_file = dir.path().join("agy-version");
        std::fs::write(&version_file, "1.4.7\n").unwrap();
        let agy = dir.path().join("agy");
        std::fs::write(
            &agy,
            format!(
                "#!/bin/sh\nPATH=\"/usr/bin:/bin${{PATH:+:$PATH}}\"\nif [ \"$1\" = \"--version\" ]; then\n  cat \"{}\"\n  exit 0\nfi\nif [ \"$1\" = \"update\" ]; then\n  if [ -n \"$AGY_CLI_DISABLE_AUTO_UPDATE\" ]; then\n    echo 'AGY_CLI_DISABLE_AUTO_UPDATE must not be set' >&2\n    exit 9\n  fi\n  printf '1.5.17\\n' > \"{}\"\n  echo 'Updated to 1.5.17'\n  exit 0\nfi\nexit 2\n",
                version_file.display(),
                version_file.display(),
            ),
        )
        .unwrap();
        std::fs::set_permissions(&agy, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut core = crate::test_support::test_core_in(dir.path().into());
        crate::test_support::isolate_runtime_path(&core, None);
        core.runtime_shell_env
            .write()
            .unwrap()
            .vars
            .insert("AGY_CLI_DISABLE_AUTO_UPDATE".into(), "true".into());
        core.sessions = crate::session::SessionManager::new(
            core.runtime_shell_env.clone(),
            core.runtime_discovery.clone(),
            Arc::new(crate::session::pty_runtime::PtyRuntime::new()),
        );
        crate::db::set_runtime_override(&core.db, "antigravity", Some(agy.to_str().unwrap()))
            .unwrap();
        assert_eq!(
            crate::ops::runtime::runtime_probe_version(&core, Runtime::Antigravity).as_deref(),
            Some("1.4.7")
        );
        let before = crate::ops::runtime::runtime_status_list(&core)
            .unwrap()
            .runtimes
            .into_iter()
            .find(|r| r.name == Runtime::Antigravity)
            .unwrap();
        assert!(before.below_minimum);

        let started = start(&core, Runtime::Antigravity).unwrap();
        assert_eq!(started.kind, RuntimeTaskKind::Update);
        assert_eq!(started.from_version.as_deref(), Some("1.4.7"));

        let finished = await_task(&core, RuntimeUpdateState::Succeeded);
        assert_eq!(finished.installed_version.as_deref(), Some("1.5.17"));

        let after = crate::ops::runtime::runtime_status_list(&core)
            .unwrap()
            .runtimes
            .into_iter()
            .find(|r| r.name == Runtime::Antigravity)
            .unwrap();
        assert_eq!(after.installed_version.as_deref(), Some("1.5.17"));
        assert!(!after.below_minimum);
        core.sessions.drain_terminal_workers();
    }
}
