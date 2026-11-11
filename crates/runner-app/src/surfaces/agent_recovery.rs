use gpui::prelude::*;
use gpui::{App, Context, WeakEntity, Window};
use runner_app::ui::SessionOverlay;
use runner_core::protocol::Runtime;

use super::front_page::{FrontPageClient, FrontPageTarget};
use crate::i18n::{text, tf};
use crate::NativeRoot;

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
        let task = self.target_task(&client.target, runtime);
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
                            ("runtime", gate.runtime.display_name().into()),
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
        Some(
            SessionOverlay::ended(
                format!("agent-recovery-{session}"),
                reason,
                move |window, cx| {
                    let _ = owner.update(cx, |root, cx| {
                        if answer {
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
            .label(if answer {
                text("agent_updates.answer").into()
            } else {
                tf("agent_updates.update_action", &[("runtime", name.into())])
            })
            .primary_debug_selector(format!("AGENT_RECOVERY_UPDATE:{session}"))
            .subtitle_debug_selector(cause_selector)
            .actions_visible(!waiting || answer, false),
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
}
