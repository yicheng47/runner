use super::*;

#[test]
fn normalized_status_snapshot_wait_gate_resolution_and_bridge_failure() {
    use crate::session::status::{HumanInteraction, WaitReason};
    #[derive(Default)]
    struct DeliveryEvents(Mutex<Vec<router::SessionDeliveryEvent>>);
    impl router::SessionDeliveryListener for DeliveryEvents {
        fn session_delivery_event(&self, _: &str, event: router::SessionDeliveryEvent) {
            self.0.lock().unwrap().push(event);
        }
    }
    let manager = mgr_with_fake(None, fake_runtime());
    install_test_session_handle(&manager, "status");
    let delivered = Arc::new(DeliveryEvents::default());
    let listener: Arc<dyn router::SessionDeliveryListener> = delivered.clone();
    manager.register_delivery_listener("status", Arc::downgrade(&listener));
    let events = capture();
    let mut observation = AgentObservation {
        activity: Activity::Working,
        source: ObservationSource::Hook,
        ..Default::default()
    };
    manager.publish_observation("status", observation.clone(), events.as_ref());
    let token = match manager.reserve_delivery("status").unwrap() {
        router::DeliveryReservation::Ready(token) => token,
        other => panic!("{other:?}"),
    };
    manager.finish_delivery("status", token);
    observation.interactions.push(HumanInteraction {
        id: "wait".into(),
        reason: WaitReason::Answer,
        owners: vec!["tool".into()],
        since: 1,
    });
    manager.publish_observation("status", observation.clone(), events.as_ref());
    assert_eq!(
        manager.reserve_delivery("status").unwrap(),
        router::DeliveryReservation::HumanInteraction
    );
    manager.mark_status_viewed(&["status".into()]);
    assert!(manager.status_snapshot()["status"].observation.needs_you());
    assert!(!manager.note_forwarder_transition("status", SessionActivityState::Idle, "forwarder"));
    assert_eq!(
        manager.status_snapshot()["status"].observation.source,
        ObservationSource::Hook
    );
    manager
        .session_state("status")
        .unwrap()
        .lock()
        .unwrap()
        .local_input_pending = true;
    observation.interactions.clear();
    observation.activity = Activity::Ready;
    observation.outcome = Some(TurnOutcome::Interrupted);
    manager.publish_observation("status", observation, events.as_ref());
    assert_eq!(
        manager.reserve_delivery("status").unwrap(),
        router::DeliveryReservation::PendingInput
    );
    assert!(delivered
        .0
        .lock()
        .unwrap()
        .contains(&router::SessionDeliveryEvent::InputCleared));
    manager
        .session_state("status")
        .unwrap()
        .lock()
        .unwrap()
        .local_input_pending = false;
    assert!(matches!(
        manager.reserve_delivery("status").unwrap(),
        router::DeliveryReservation::Ready(_)
    ));
    manager.finish_delivery("status", token);
    manager.status_bridge_failed("status", events.as_ref());
    let status = manager.status_snapshot()["status"].clone();
    assert_eq!(status.observation.source, ObservationSource::Baseline);
    assert_eq!(status.observation.activity, Activity::Idle);
    assert!(!status.observation.needs_you());
}

#[test]
fn failure_attention_is_transient_and_interruption_never_records_unread() {
    let core = crate::test_support::test_core();
    core.db.get().unwrap().execute("INSERT INTO sessions(id, status, agent_runtime) VALUES ('status-detail', 'running', 'claude-code')", []).unwrap();
    install_test_session_handle(&core.sessions, "status-detail");
    let events = core.session_events();
    let mut observation = AgentObservation {
        activity: Activity::Working,
        source: ObservationSource::Hook,
        ..Default::default()
    };
    core.sessions
        .publish_observation("status-detail", observation.clone(), &events);

    observation.activity = Activity::Ready;
    observation.outcome = Some(TurnOutcome::Interrupted);
    core.sessions
        .publish_observation("status-detail", observation.clone(), &events);
    let interrupted = core.sessions.agent_status("status-detail");
    assert!(interrupted.failed_since.is_none());
    assert!(interrupted.unread_since.is_none());
    assert!(!crate::repo::session_attention::any_unread(
        &core.db.get().unwrap(),
        &["status-detail".into()]
    )
    .unwrap());

    observation.outcome = Some(TurnOutcome::Completed);
    core.sessions
        .publish_observation("status-detail", observation.clone(), &events);
    assert!(core
        .sessions
        .agent_status("status-detail")
        .failed_since
        .is_none());

    observation.outcome = Some(TurnOutcome::Failed);
    core.sessions
        .publish_observation("status-detail", observation.clone(), &events);
    let failed = core.sessions.agent_status("status-detail");
    assert!(failed.failed_since.is_some());
    assert!(failed.unread_since.is_none());

    observation.activity = Activity::Working;
    observation.outcome = None;
    observation.detail = Some(crate::session::status::WorkDetail::CompactingContext);
    core.sessions
        .publish_observation("status-detail", observation.clone(), &events);
    assert!(core
        .sessions
        .agent_status("status-detail")
        .failed_since
        .is_none());
    assert!(!core
        .sessions
        .take_completion_armed(&["status-detail".into()]));
    observation.activity = Activity::Ready;
    observation.outcome = Some(TurnOutcome::Failed);
    observation.detail = None;
    core.sessions
        .publish_observation("status-detail", observation.clone(), &events);
    assert!(core
        .sessions
        .agent_status("status-detail")
        .failed_since
        .is_some());

    core.sessions.mark_status_viewed(&["status-detail".into()]);
    let acknowledged = core.sessions.agent_status("status-detail");
    assert!(acknowledged.failed_since.is_none());
    assert_eq!(acknowledged.observation.outcome, Some(TurnOutcome::Failed));
    observation.activity = Activity::Working;
    observation.outcome = None;
    observation.detail = Some(crate::session::status::WorkDetail::CompactingContext);
    core.sessions
        .publish_observation("status-detail", observation.clone(), &events);
    assert!(!core
        .sessions
        .take_completion_armed(&["status-detail".into()]));
    observation.activity = Activity::Ready;
    observation.outcome = Some(TurnOutcome::Failed);
    observation.detail = None;
    core.sessions
        .publish_observation("status-detail", observation.clone(), &events);
    assert!(core
        .sessions
        .agent_status("status-detail")
        .failed_since
        .is_none());

    observation.activity = Activity::Idle;
    core.sessions
        .publish_observation("status-detail", observation.clone(), &events);
    assert!(core
        .sessions
        .agent_status("status-detail")
        .failed_since
        .is_none());

    observation.activity = Activity::Working;
    observation.outcome = None;
    core.sessions
        .publish_observation("status-detail", observation.clone(), &events);
    assert!(core
        .sessions
        .agent_status("status-detail")
        .failed_since
        .is_none());

    observation.source = ObservationSource::Baseline;
    observation.detail = Some(crate::session::status::WorkDetail::UsingTools);
    core.sessions
        .publish_observation("status-detail", observation.clone(), &events);
    assert_eq!(
        core.sessions
            .agent_status("status-detail")
            .observation
            .detail,
        None
    );

    {
        let state = core.sessions.session_state("status-detail").unwrap();
        let mut state = state.lock().unwrap();
        state.handle = None;
        state.status.lifecycle = Lifecycle::Stopped;
    }
    observation.activity = Activity::Ready;
    observation.outcome = Some(TurnOutcome::Failed);
    core.sessions
        .publish_observation("status-detail", observation, &events);
    assert!(core
        .sessions
        .agent_status("status-detail")
        .failed_since
        .is_none());
}

#[test]
fn bridge_loss_and_unavailable_observations_never_manufacture_a_completion() {
    let core = crate::test_support::test_core();
    core.db.get().unwrap().execute("INSERT INTO sessions(id, status, agent_runtime) VALUES ('status', 'running', 'claude-code')", []).unwrap();
    install_test_session_handle(&core.sessions, "status");
    let events = core.session_events();
    let working = AgentObservation {
        activity: Activity::Working,
        source: ObservationSource::Hook,
        ..Default::default()
    };
    core.sessions
        .publish_observation("status", working.clone(), &events);
    core.sessions.publish_observation(
        "status",
        AgentObservation {
            activity: Activity::Unavailable,
            source: ObservationSource::Hook,
            ..Default::default()
        },
        &events,
    );
    assert!(core.sessions.agent_status("status").unread_since.is_none());
    core.sessions
        .publish_observation("status", working, &events);
    core.sessions
        .note_forwarder_transition("status", SessionActivityState::Idle, "forwarder");
    core.sessions.status_bridge_failed("status", &events);
    assert!(core.sessions.agent_status("status").unread_since.is_none());
    assert!(!core.sessions.take_completion_armed(&["status".into()]));
    assert!(!crate::repo::session_attention::any_unread(
        &core.db.get().unwrap(),
        &["status".into()]
    )
    .unwrap());
}

#[test]
fn hook_session_start_does_not_consume_completion_arming() {
    let core = crate::test_support::test_core();
    core.db.get().unwrap().execute("INSERT INTO sessions(id, status, agent_runtime) VALUES ('status', 'running', 'claude-code'), ('pi-status', 'running', 'pi')", []).unwrap();
    for session_id in ["status", "pi-status"] {
        install_test_session_handle(&core.sessions, session_id);
        core.sessions.arm_completion(session_id);
    }
    let events = core.session_events();

    core.sessions.publish_observation(
        "status",
        AgentObservation {
            activity: Activity::Idle,
            source: ObservationSource::Hook,
            ..Default::default()
        },
        &events,
    );
    assert!(core.sessions.agent_status("status").unread_since.is_none());

    core.sessions.publish_observation(
        "status",
        AgentObservation {
            activity: Activity::Working,
            source: ObservationSource::Hook,
            ..Default::default()
        },
        &events,
    );
    core.sessions.publish_observation(
        "status",
        AgentObservation {
            activity: Activity::Ready,
            source: ObservationSource::Hook,
            outcome: Some(TurnOutcome::Completed),
            ..Default::default()
        },
        &events,
    );
    assert!(core.sessions.agent_status("status").unread_since.is_some());
    assert!(!core.sessions.take_completion_armed(&["status".into()]));

    core.sessions.publish_observation(
        "pi-status",
        AgentObservation {
            activity: Activity::Working,
            source: ObservationSource::Hook,
            ..Default::default()
        },
        &events,
    );
    core.sessions.publish_observation(
        "pi-status",
        AgentObservation {
            activity: Activity::Ready,
            source: ObservationSource::Hook,
            ..Default::default()
        },
        &events,
    );
    assert!(core
        .sessions
        .agent_status("pi-status")
        .unread_since
        .is_some());
    assert!(!core.sessions.take_completion_armed(&["pi-status".into()]));
}

#[test]
fn reserved_write_backpressure_does_not_block_status_observation() {
    let runtime = fake_runtime();
    let manager = mgr_with_fake(None, Arc::clone(&runtime));
    install_test_session_handle(&manager, "blocked-write");
    let token = match manager.reserve_delivery("blocked-write").unwrap() {
        router::DeliveryReservation::Ready(token) => token,
        other => panic!("{other:?}"),
    };
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    *runtime.write_gate.lock().unwrap() = Some(RuntimeGate {
        entered: entered_tx,
        release: release_rx,
    });
    let writer = Arc::clone(&manager);
    let write =
        std::thread::spawn(move || writer.inject_reserved("blocked-write", token, b"nudge"));
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let observer = Arc::clone(&manager);
    let (observed_tx, observed_rx) = std::sync::mpsc::channel();
    let observe = std::thread::spawn(move || {
        observer.publish_observation(
            "blocked-write",
            AgentObservation {
                activity: Activity::Working,
                source: ObservationSource::Hook,
                interactions: vec![crate::session::status::HumanInteraction {
                    id: "approval".into(),
                    reason: crate::session::status::WaitReason::Approval,
                    owners: vec!["tool".into()],
                    since: 1,
                }],
                ..Default::default()
            },
            capture().as_ref(),
        );
        observed_tx
            .send(observer.agent_status("blocked-write"))
            .unwrap();
    });
    let status = observed_rx.recv_timeout(Duration::from_secs(2));
    release_tx.send(()).unwrap();
    assert!(write.join().unwrap().unwrap());
    observe.join().unwrap();
    assert!(status.unwrap().observation.needs_you());
    assert!(!manager
        .inject_reserved("blocked-write", token, b"\r")
        .unwrap());
    assert_eq!(runtime.bytes_writes().len(), 1);
    manager.finish_delivery("blocked-write", token);
}
