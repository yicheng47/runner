use gpui::prelude::*;
use gpui::{App, Context, WeakEntity, Window};
use runner_app::ui::SessionOverlay;
use runner_core::protocol::Runtime;

use super::front_page::{FrontPageClient, FrontPageTarget};
use crate::i18n::{text, tf};
use crate::NativeRoot;

pub(crate) fn runtime_updatable(runtime: Runtime) -> bool {
    runner_core::protocol::runtime_metadata::catalog_for(runtime)
        .is_some_and(|catalog| !catalog.update_args.is_empty())
}

pub(crate) fn installed_runtime_name(runtime: Runtime) -> &'static str {
    if runtime == Runtime::OpenCode {
        "OpenCode"
    } else {
        runtime.display_name()
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum RecoveryAction {
    #[default]
    Resume,
    Restart,
}

pub(crate) struct SessionGate {
    pub(crate) gate: runner_core::protocol::RuntimeVersionGate,
    pub(crate) action: RecoveryAction,
}

pub(crate) struct WaitingSession {
    pub(crate) runtime: Runtime,
    client: FrontPageClient,
    size: (u16, u16),
    action: RecoveryAction,
}

impl NativeRoot {
    pub(crate) fn record_session_gate(
        &mut self,
        target: &FrontPageTarget,
        session: &str,
        gate: Option<runner_core::protocol::RuntimeVersionGate>,
        action: RecoveryAction,
        succeeded: bool,
    ) {
        let key = (target.clone(), session.to_owned());
        if succeeded {
            self.agent_tasks.gates.remove(&key);
        } else if let Some(gate) = gate {
            self.agent_tasks
                .gates
                .insert(key, SessionGate { gate, action });
        }
    }

    pub(crate) fn clear_unblocked_gates(
        &mut self,
        target: &FrontPageTarget,
        supported: &[Runtime],
    ) {
        self.agent_tasks
            .gates
            .retain(|(host, _), entry| host != target || !supported.contains(&entry.gate.runtime));
    }

    pub(crate) fn retire_obsolete_startup_errors(
        &mut self,
        target: &FrontPageTarget,
        verified_supported: &[Runtime],
        cx: &mut Context<Self>,
    ) {
        if *target != FrontPageTarget::Local || verified_supported.is_empty() {
            return;
        }
        let changed = self.app_store.update(cx, |store, cx| {
            let mut cleared = false;
            for status in store.session_statuses.values_mut() {
                let obsolete = status.startup_error.as_ref().is_some_and(|error| {
                    verified_supported.contains(&error.runtime)
                        && runner_core::protocol::runtime_metadata::catalog_for(error.runtime)
                            .is_some_and(|catalog| {
                                catalog.min_version.is_some() && catalog.update_args.is_empty()
                            })
                });
                if obsolete {
                    status.startup_error = None;
                    cleared = true;
                }
            }
            if cleared {
                cx.notify();
            }
            cleared
        });
        if changed {
            cx.notify();
        }
    }

    pub(crate) fn agent_recovery_overlay(
        &self,
        pane: &str,
        session: &str,
        runtime: Runtime,
        owner: WeakEntity<Self>,
        cx: &App,
    ) -> Option<SessionOverlay> {
        let client = self.front_page_client(cx);
        let key = (client.target.clone(), session.to_owned());
        let gate = self
            .agent_tasks
            .gates
            .get(&key)
            .map(|entry| &entry.gate)
            .or_else(|| {
                (!self.front_page_client(cx).is_remote())
                    .then(|| self.agent_tasks.minimums.get(&runtime))
                    .flatten()
            });
        let startup = if self.front_page_client(cx).is_remote() {
            self.remote_client.startup_error(session, cx)
        } else {
            self.app_store
                .read(cx)
                .session_statuses
                .get(session)
                .and_then(|status| status.startup_error.clone())
        };
        if gate.is_none() && startup.is_none() {
            return None;
        }
        let updatable = runtime_updatable(runtime);
        let remote = client.is_remote();
        let task = updatable
            .then(|| self.target_task(&client.target, runtime))
            .flatten();
        let answer = task.is_some_and(|task| {
            task.state == runner_core::protocol::RuntimeUpdateState::MayNeedInput
        });
        let output_task = task.cloned();
        let waiting = self.agent_tasks.waiting_sessions.contains_key(&key)
            && task.is_some_and(|task| task.state.active());
        let name = runtime.display_name();
        let title = if waiting {
            tf("agent_updates.waiting", &[("runtime", name.into())])
        } else if startup.is_some() {
            tf("agent_updates.flags_title", &[("runtime", name.into())])
        } else {
            tf("agent_updates.too_old", &[("runtime", name.into())])
        };
        let reason = task
            .and_then(|task| task.message.clone())
            .or_else(|| {
                gate.map(|gate| {
                    tf(
                        "agent_updates.minimum_reason",
                        &[
                            ("minimum", gate.min_version.clone()),
                            ("runtime", installed_runtime_name(gate.runtime).into()),
                            ("installed", gate.installed_version.clone()),
                        ],
                    )
                })
            })
            .or_else(|| startup.map(|error| error.message.clone()))
            .unwrap_or_default();
        let pane = pane.to_owned();
        let id = session.to_owned();
        let cause_selector = format!("AGENT_RECOVERY_CAUSE:{reason}");
        let primary_selector = if updatable {
            format!("AGENT_RECOVERY_UPDATE:{session}")
        } else {
            format!("AGENT_RECOVERY_OPEN_SETTINGS:{session}")
        };
        Some(
            SessionOverlay::ended(
                format!("agent-recovery-{session}"),
                reason,
                move |window, cx| {
                    let _ = owner.update(cx, |root, cx| {
                        if !updatable {
                            if remote {
                                return;
                            }
                            root.start_chat_modal = None;
                            root.enter_settings_route(Some("agents"), window, cx);
                        } else if answer {
                            if let Some(task) = &output_task {
                                root.show_target_task(
                                    task.clone(),
                                    Some(client.clone()),
                                    window,
                                    cx,
                                );
                            }
                        } else {
                            root.start_session_update(&pane, &id, runtime, window, cx);
                        }
                    });
                },
                |_, _| {},
            )
            .title(title)
            .label(if !updatable {
                text("ui.open_settings").into()
            } else if answer {
                text("agent_updates.answer").into()
            } else {
                tf("agent_updates.update_action", &[("runtime", name.into())])
            })
            .primary_debug_selector(primary_selector)
            .subtitle_debug_selector(cause_selector)
            .actions_visible(
                if updatable {
                    !waiting || answer
                } else {
                    !remote
                },
                false,
            ),
        )
    }

    fn start_session_update(
        &mut self,
        pane: &str,
        session: &str,
        runtime: Runtime,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !runtime_updatable(runtime) {
            return;
        }
        let client = self.front_page_client(cx);
        let size = self
            .attached
            .get(session)
            .map(|chat| chat.terminal.size())
            .or_else(|| {
                self.front_page_layout()
                    .map(|layout| self.estimated_terminal_size(layout, pane, window, cx))
            })
            .unwrap_or((80, 24));
        let key = (client.target.clone(), session.into());
        let action = self
            .agent_tasks
            .gates
            .get(&key)
            .map_or(RecoveryAction::Resume, |entry| entry.action);
        self.agent_tasks.waiting_sessions.insert(
            key,
            WaitingSession {
                runtime,
                client: client.clone(),
                size,
                action,
            },
        );
        self.start_target_update(client, runtime, window, cx);
    }

    pub(crate) fn retry_gated_sessions(
        &mut self,
        runtime: Runtime,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.retry_remote_gated_sessions(
            runtime,
            &super::front_page::FrontPageTarget::Local,
            window,
            cx,
        );
    }

    pub(crate) fn retry_remote_gated_sessions(
        &mut self,
        runtime: Runtime,
        target_host: &super::front_page::FrontPageTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids = self
            .agent_tasks
            .waiting_sessions
            .iter()
            .filter(|(_, wait)| wait.runtime == runtime && &wait.client.target == target_host)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for key in ids {
            let wait = self.agent_tasks.waiting_sessions.remove(&key).unwrap();
            let target = key.1.clone();
            let action = wait.action;
            let work = cx.background_spawn(async move {
                match action {
                    RecoveryAction::Resume => wait.client.resume(&target, wait.size),
                    RecoveryAction::Restart => wait.client.restart(&target, wait.size),
                }
            });
            cx.spawn_in(window, async move |owner, cx| {
                let result = work.await;
                let _ = owner.update_in(cx, |root, window, cx| {
                    match result {
                        Ok(session) => {
                            root.agent_tasks.gates.remove(&key);
                            if matches!(key.0, FrontPageTarget::Local) {
                                root.refresh_sessions(cx);
                                let _ = root.ensure_active_tab_attached(window, cx);
                            } else if root.remote_client.reopen_sessions(&key.0, vec![session]) {
                                root.sync_remote_state(cx);
                            }
                        }
                        Err(error) => {
                            if let Some(gate) = error.version_gate {
                                root.agent_tasks
                                    .gates
                                    .insert(key, SessionGate { gate, action });
                            } else {
                                root.chat_error = Some(error.message.into());
                            }
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surfaces::start_chat::{tests::modal_harness, ChatMode};
    use runner_core::protocol::{AgentStatus, Lifecycle, RuntimeVersionGate};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    struct Probe {
        inner: runner_daemon::daemon::InProcessTransport,
        supported: Arc<AtomicBool>,
        unknown: bool,
        startup: Option<runner_core::protocol::StartupError>,
    }
    impl runner_core::protocol::Transport for Probe {
        fn call(
            &self,
            request: runner_core::protocol::Request,
        ) -> runner_core::protocol::ClientResult<runner_core::protocol::Response> {
            if matches!(
                request,
                runner_core::protocol::Request::session_status_snapshot { .. }
            ) {
                if let Some(error) = &self.startup {
                    return Ok(runner_core::protocol::Response::session_status_snapshot(
                        Ok([(
                            "failed-start".into(),
                            AgentStatus {
                                lifecycle: Lifecycle::Stopped,
                                startup_error: Some(error.clone()),
                                ..Default::default()
                            },
                        )]
                        .into()),
                    ));
                }
            }
            let mut response = self.inner.call(request)?;
            if let runner_core::protocol::Response::runtime_status_list(Ok(status)) = &mut response
            {
                let supported = self.supported.load(Ordering::Relaxed);
                let runtime = status
                    .runtimes
                    .iter_mut()
                    .find(|runtime| runtime.name == Runtime::Codex)
                    .unwrap();
                runtime.installed_version =
                    (!self.unknown).then(|| if supported { "0.167.0" } else { "0.147.0" }.into());
                runtime.below_minimum = !supported;
                runtime.min_version = Some("0.149.0".into());
            }
            Ok(response)
        }
        fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
            self.inner.subscribe()
        }
    }

    #[test]
    fn a_supported_probe_after_settings_update_retires_only_the_local_gate() {
        let mut harness = modal_harness(1200., 900., Vec::new(), Vec::new(), ChatMode::Runtime);
        let supported = Arc::new(AtomicBool::new(false));
        let remote = FrontPageTarget::Remote {
            computer: "other".into(),
            fingerprint: "fp".into(),
            generation: 1,
        };
        harness.act(|root, _, cx| {
            let gate = RuntimeVersionGate {
                runtime: Runtime::Codex,
                installed_version: "0.147.0".into(),
                min_version: "0.149.0".into(),
            };
            root.record_session_gate(
                &FrontPageTarget::Local,
                "same-id",
                Some(gate.clone()),
                RecoveryAction::Resume,
                false,
            );
            root.record_session_gate(
                &remote,
                "same-id",
                Some(gate),
                RecoveryAction::Resume,
                false,
            );
            root.app_store.update(cx, |store, _| {
                store.client = runner_core::protocol::DaemonClient::new(Arc::new(Probe {
                    inner: runner_daemon::daemon::InProcessTransport(store.test_core.clone()),
                    supported: supported.clone(),
                    unknown: false,
                    startup: None,
                }));
            });
            root.app_store
                .read(cx)
                .test_core
                .events
                .emit("runtime/changed", &());
        });
        harness.read_root(|root, _| assert_eq!(root.agent_tasks.gates.len(), 2));
        supported.store(true, Ordering::Relaxed);
        harness.act(|root, _, cx| {
            root.app_store
                .read(cx)
                .test_core
                .events
                .emit("runtime/changed", &())
        });
        harness.read_root(|root, _| {
            assert!(!root
                .agent_tasks
                .gates
                .contains_key(&(FrontPageTarget::Local, "same-id".into())));
            assert!(!root.agent_tasks.minimums.contains_key(&Runtime::Codex));
            assert!(root
                .agent_tasks
                .gates
                .contains_key(&(remote, "same-id".into())));
        });
    }

    #[test]
    fn an_unknown_probe_also_releases_an_obsolete_known_version_gate() {
        let mut harness = modal_harness(1200., 900., Vec::new(), Vec::new(), ChatMode::Runtime);
        harness.act(|root, _, cx| {
            root.record_session_gate(
                &FrontPageTarget::Local,
                "old-refusal",
                Some(RuntimeVersionGate {
                    runtime: Runtime::Codex,
                    installed_version: "0.147.0".into(),
                    min_version: "0.149.0".into(),
                }),
                RecoveryAction::Resume,
                false,
            );
            root.app_store.update(cx, |store, _| {
                store.client = runner_core::protocol::DaemonClient::new(Arc::new(Probe {
                    inner: runner_daemon::daemon::InProcessTransport(store.test_core.clone()),
                    supported: Arc::new(AtomicBool::new(true)),
                    unknown: true,
                    startup: None,
                }))
            });
            root.app_store
                .read(cx)
                .test_core
                .events
                .emit("runtime/changed", &());
        });
        harness.read_root(|root, _| assert!(root.agent_tasks.gates.is_empty()));
    }

    #[test]
    fn classified_stopped_cards_show_the_bounded_configuration_cause() {
        for output in [
            "Error loading config.toml: unknown variant `mcp_tool`, expected one of `command`, `prompt`, `agent` in `hooks`",
            "Error loading config.toml: invalid type: integer, expected a string in hooks.command",
        ] {
            let mut harness = modal_harness(1200., 900., Vec::new(), Vec::new(), ChatMode::Runtime);
            let error = runner_daemon::runtimes::adapter(Runtime::Codex).classify_startup_error(output).unwrap();
            let message = error.message.clone();
            harness.act(|root, _, cx| {
                root.start_chat_modal = None;
                let core = root.app_store.read(cx).test_core.clone();
                root.app_store.update(cx, |store, _| store.client = runner_core::protocol::DaemonClient::new(Arc::new(Probe {
                    inner: runner_daemon::daemon::InProcessTransport(core.clone()), supported: Arc::new(AtomicBool::new(true)), unknown: false, startup: Some(error.clone()),
                })));
                core.db.get().unwrap().execute("INSERT INTO sessions(id, status, agent_runtime, agent_command) VALUES ('failed-start', 'stopped', 'codex', 'codex')", []).unwrap();
                let mut layout = crate::PaneLayout::single(Some("failed-start"), &["failed-start".into()]);
                layout.id = runner_core::event_log::UlidGen::new().next().unwrap();
                root.front_page_client(cx).launcher_layout(&layout, None).unwrap();
                root.refresh_sessions(cx);
                root.reload_tabs(cx).unwrap();
                root.tabs.activate_session("failed-start");
                root.set_route(crate::AppRoute::Chat, cx);
                root.agent_tasks.minimums.clear();
                root.app_store.update(cx, |store, cx| {
                    store.session_statuses.insert("failed-start".into(), AgentStatus { lifecycle: Lifecycle::Stopped, startup_error: Some(error), ..Default::default() });
                    cx.notify();
                });
                cx.notify();
            });
            assert!(harness.bounds(&format!("AGENT_RECOVERY_CAUSE:{message}")).is_some(), "{message}");
            assert!(harness.bounds("AGENT_RECOVERY_UPDATE:failed-start").is_some());
        }
    }

    #[test]
    fn non_updatable_runtime_recovery_offers_open_settings_without_starting_update() {
        let mut harness = modal_harness(1200., 900., Vec::new(), Vec::new(), ChatMode::Runtime);
        harness.act(|root, _, cx| {
            root.start_chat_modal = None;
            let core = root.app_store.read(cx).test_core.clone();
            core.db
                .get()
                .unwrap()
                .execute(
                    "INSERT INTO sessions(id, status, agent_runtime, agent_command) VALUES ('opencode-v1', 'stopped', 'opencode', 'opencode')",
                    [],
                )
                .unwrap();
            let mut layout =
                crate::PaneLayout::single(Some("opencode-v1"), &["opencode-v1".into()]);
            layout.id = runner_core::event_log::UlidGen::new().next().unwrap();
            root.front_page_client(cx)
                .launcher_layout(&layout, None)
                .unwrap();
            root.refresh_sessions(cx);
            root.reload_tabs(cx).unwrap();
            root.tabs.activate_session("opencode-v1");
            root.set_route(crate::AppRoute::Chat, cx);
            root.record_session_gate(
                &FrontPageTarget::Local,
                "opencode-v1",
                Some(RuntimeVersionGate {
                    runtime: Runtime::OpenCode,
                    installed_version: "1.19.5".into(),
                    min_version: "2.0.0".into(),
                }),
                RecoveryAction::Resume,
                false,
            );
            cx.notify();
        });
        assert!(harness
            .bounds(
                "AGENT_RECOVERY_CAUSE:Runner needs 2.0.0 or newer. OpenCode 1.19.5 is installed."
            )
            .is_some());
        assert!(harness
            .bounds("AGENT_RECOVERY_UPDATE:opencode-v1")
            .is_none());
        let button = harness
            .bounds("AGENT_RECOVERY_OPEN_SETTINGS:opencode-v1")
            .expect("expected Open Settings action");
        harness
            .visual
            .simulate_click(button.center(), gpui::Modifiers::default());
        harness.visual.run_until_parked();
        harness.read_root(|root, _| {
            assert_eq!(root.route, crate::AppRoute::Settings);
            assert_eq!(
                root.settings_page.pane,
                crate::surfaces::settings::page::SettingsPane::Agents
            );
            assert!(root.agent_tasks.tasks.is_empty());
            assert!(root.agent_tasks.waiting_sessions.is_empty());
        });
    }

    #[cfg(unix)]
    #[test]
    fn opencode_startup_error_retires_on_supported_refresh_and_allows_resume() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        let log = dir.path().join("resumed-args");
        std::fs::create_dir_all(&bin).unwrap();
        let opencode = bin.join("opencode");
        // Phase 1: unknown-version wrapper that fails on --standalone with CLI help.
        std::fs::write(
            &opencode,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'opencode dev'; exit 0; fi\necho 'Commands:'\necho 'Options:'\necho '  -h, --help'\necho 'Unknown argument: standalone'\nsleep 0.05\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&opencode, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut harness = modal_harness(1200., 900., Vec::new(), Vec::new(), ChatMode::Runtime);
        harness.act(|root, _, cx| {
            root.start_chat_modal = None;
            let core = root.app_store.read(cx).test_core.clone();
            core.runtime_shell_env.write().unwrap().path = Some(bin.display().to_string());
            {
                let mut discovery = core.runtime_discovery.write().unwrap();
                discovery.checking = false;
                discovery.result = Some(runner_daemon::shell_path::DiscoveryResult {
                    shell: Some("/bin/zsh".into()),
                    outcome: runner_daemon::shell_path::DiscoveryOutcome::Ok,
                    duration_ms: 10,
                    env: runner_daemon::shell_path::LoginShellEnv::default(),
                });
            }
            let started = runner_daemon::ops::session::session_start_runtime(
                &core,
                "opencode",
                runner_core::protocol::ProjectScope::Root,
                Some(dir.path().to_str().unwrap().to_owned()),
                Some(80),
                Some(24),
                None,
                None,
            )
            .unwrap();
            let id = started.session.id;
            let mut layout = crate::PaneLayout::single(Some(&id), std::slice::from_ref(&id));
            layout.id = runner_core::event_log::UlidGen::new().next().unwrap();
            root.front_page_client(cx)
                .launcher_layout(&layout, None)
                .unwrap();
            root.refresh_sessions(cx);
            root.reload_tabs(cx).unwrap();
            root.tabs.activate_session(&id);
            root.set_route(crate::AppRoute::Chat, cx);
            cx.notify();
        });
        let session_id = harness.read_root(|root, cx| {
            root.app_store
                .read(cx)
                .test_core
                .db
                .get()
                .unwrap()
                .query_row("SELECT id FROM sessions LIMIT 1", [], |row| {
                    row.get::<_, String>(0)
                })
                .unwrap()
        });

        let open_settings_selector = format!("AGENT_RECOVERY_OPEN_SETTINGS:{session_id}");
        let update_selector = format!("AGENT_RECOVERY_UPDATE:{session_id}");
        let resume_selector = format!("BUTTON:ended-{session_id}-resume");

        // Waiting for the early --standalone failure populates StartupError -> Open Settings.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while harness.bounds(&open_settings_selector).is_none()
            && std::time::Instant::now() < deadline
        {
            harness.visual.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        assert!(
            harness.bounds(&open_settings_selector).is_some(),
            "expected Open Settings recovery card after classified --standalone exit"
        );
        assert!(harness.bounds(&update_selector).is_none());
        assert!(harness.bounds(&resume_selector).is_none());

        // Phase 2: upgrade the executable to OpenCode v2 and refresh runtime discovery.
        std::fs::write(
            &opencode,
            format!(
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo '2.0.29'; exit 0; fi\nprintf '%s\\n' \"$@\" > \"{}\"\nsleep 2\n",
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&opencode, std::fs::Permissions::from_mode(0o755)).unwrap();

        harness.act(|root, _, cx| {
            let core = root.app_store.read(cx).test_core.clone();
            assert_eq!(
                runner_daemon::ops::runtime::runtime_probe_version(&core, Runtime::OpenCode)
                    .as_deref(),
                Some("2.0.29")
            );
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while harness.bounds(&resume_selector).is_none() && std::time::Instant::now() < deadline {
            harness.visual.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert!(harness.bounds(&open_settings_selector).is_none());
        let resume = harness
            .bounds(&resume_selector)
            .expect("supported refresh should retire OpenCode StartupError and restore Resume");
        harness
            .visual
            .simulate_click(resume.center(), gpui::Modifiers::default());
        harness.visual.run_until_parked();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !log.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            log.exists(),
            "resuming {session_id} did not launch OpenCode v2"
        );
        let args = std::fs::read_to_string(&log).unwrap();
        assert!(args.lines().any(|line| line == "--standalone"), "{args}");
        harness.read_root(|root, cx| {
            let status = &root.app_store.read(cx).test_core.sessions.status_snapshot()[&session_id];
            assert_eq!(status.startup_error, None);
            assert!(root.agent_tasks.tasks.is_empty());
            assert!(root.agent_tasks.waiting_sessions.is_empty());
        });
    }
}
