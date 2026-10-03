use super::*;

fn now() -> Now {
    Now {
        monotonic: Instant::now(),
        wall_millis: 1000,
    }
}

fn transition(
    model: &mut SessionModel,
    state: SessionActivityState,
    source: StatusSource,
) -> Effects {
    model.apply(
        SessionEvent::Transition {
            state,
            source,
            live: true,
        },
        now(),
    )
}

fn observe(model: &mut SessionModel, observation: AgentObservation, at: Now) -> Effects {
    model.apply(
        SessionEvent::Observation {
            observation,
            live: true,
        },
        at,
    )
}

fn hook(activity: Activity) -> AgentObservation {
    AgentObservation {
        activity,
        source: ObservationSource::Hook,
        ..Default::default()
    }
}

#[test]
fn rule_1_baseline_transitions_publish_estimated_activity() {
    let mut model = SessionModel::default();
    transition(&mut model, SessionActivityState::Busy, StatusSource::Spawn);
    assert_eq!(model.status().observation.activity, Activity::Working);
    assert_eq!(
        model.status().observation.source,
        ObservationSource::Baseline
    );
    assert!(transition(
        &mut model,
        SessionActivityState::Idle,
        StatusSource::Forwarder
    )
    .publication
    .is_some());
    assert_eq!(model.status().observation.activity, Activity::Idle);
    assert!(transition(
        &mut model,
        SessionActivityState::Idle,
        StatusSource::Forwarder
    )
    .publication
    .is_none());
}

#[test]
fn rule_2_hook_authority_retains_turn_through_baseline_silence() {
    let mut model = SessionModel::default();
    observe(&mut model, hook(Activity::Working), now());
    for state in [SessionActivityState::Busy, SessionActivityState::Idle] {
        assert!(transition(&mut model, state, StatusSource::Forwarder)
            .publication
            .is_none());
        assert_eq!(model.status().observation, hook(Activity::Working));
        assert_eq!(model.baseline_activity, Some(state));
    }
}

#[test]
fn rule_3_bridge_failure_publishes_the_last_baseline() {
    let mut model = SessionModel::default();
    transition(
        &mut model,
        SessionActivityState::Idle,
        StatusSource::Forwarder,
    );
    observe(&mut model, hook(Activity::Working), now());
    let fallback = model
        .apply(SessionEvent::BridgeFailed, now())
        .fallback
        .unwrap();
    assert!(!model.completion_armed);
    observe(&mut model, fallback, now());
    assert_eq!(model.status().observation.activity, Activity::Idle);
    assert_eq!(
        model.status().observation.source,
        ObservationSource::Baseline
    );
    assert!(transition(
        &mut model,
        SessionActivityState::Busy,
        StatusSource::Forwarder
    )
    .publication
    .is_some());
}

#[test]
fn rule_4_legacy_interrupts_disarm_and_provisional_escape_settles() {
    for source in [StatusSource::InputInterrupt, StatusSource::InputEscape] {
        let mut model = SessionModel::default();
        observe(&mut model, hook(Activity::Working), now());
        transition(&mut model, SessionActivityState::Idle, source);
        assert_eq!(model.status().observation.activity, Activity::Unavailable);
        assert_eq!(
            model.status().observation.outcome,
            Some(TurnOutcome::Interrupted)
        );
        assert!(!model.completion_armed);
        model.apply(SessionEvent::ArmCompletion, now());
        assert_eq!(
            model
                .apply(SessionEvent::TakeCompletion, now())
                .completion_consumed,
            source != StatusSource::InputEscape
        );
        assert!(
            transition(&mut model, SessionActivityState::Idle, StatusSource::Hook)
                .publication
                .is_some()
        );
        assert!(!model.provisional_idle);
        assert_eq!(model.status().observation.activity, Activity::Ready);
    }
}

#[test]
fn rule_5_typing_suppresses_echo_and_submit_uses_the_combined_draft_rule() {
    let mut model = SessionModel::default();
    let at = now();
    transition(
        &mut model,
        SessionActivityState::Idle,
        StatusSource::Forwarder,
    );
    model.apply(
        SessionEvent::Input {
            class: Some(LocalInputClass::SetPending),
            submitted: false,
        },
        at,
    );
    assert!(transition(
        &mut model,
        SessionActivityState::Busy,
        StatusSource::Forwarder
    )
    .publication
    .is_none());
    assert!(!model.draft_quiescent(at.monotonic));
    assert_eq!(
        model.draft_hold(at.monotonic),
        Some(crate::router::DeliveryReservation::LocalInputPending)
    );
    let effects = model.apply(
        SessionEvent::Input {
            class: Some(LocalInputClass::ClearPending),
            submitted: true,
        },
        at,
    );
    assert_eq!(
        effects.publication,
        Some((SessionActivityState::Busy, StatusSource::InputSubmit))
    );
    assert!(effects.input_cleared);
    assert_eq!(model.status().observation.activity, Activity::Working);
    model.apply(
        SessionEvent::Composer {
            observation: InputObservation {
                state: InputState::Drafting,
                since: at.monotonic,
                composing: false,
                composer_visible: false,
            },
            live: true,
        },
        at,
    );
    assert_eq!(
        model.draft_hold(at.monotonic),
        Some(crate::router::DeliveryReservation::Drafting {
            composer_visible: false
        })
    );
    let effects = model.apply(
        SessionEvent::Composer {
            observation: InputObservation {
                state: InputState::Idle,
                since: at.monotonic,
                composing: false,
                composer_visible: true,
            },
            live: true,
        },
        at,
    );
    assert!(effects.input_cleared);
    assert!(model.draft_quiescent(at.monotonic));
}

#[test]
fn rule_6_last_interaction_close_releases_delivery() {
    let mut model = SessionModel::default();
    let mut waiting = hook(Activity::Ready);
    waiting
        .interactions
        .push(super::super::status::HumanInteraction {
            id: "approval".into(),
            reason: super::super::status::WaitReason::Approval,
            owners: vec!["tool".into()],
            since: 1000,
        });
    observe(&mut model, waiting.clone(), now());
    assert_eq!(model.activity(), Some(SessionActivityState::Busy));
    assert!(!observe(&mut model, waiting, now()).input_cleared);
    assert!(observe(&mut model, hook(Activity::Working), now()).input_cleared);
    assert!(!model.status().observation.needs_you());
}

#[test]
fn rule_7_failure_timestamp_survives_compaction_and_acknowledgement() {
    for viewed in [false, true] {
        let mut model = SessionModel::default();
        let at = now();
        let mut failed = hook(Activity::Ready);
        failed.outcome = Some(TurnOutcome::Failed);
        observe(&mut model, failed.clone(), at);
        assert_eq!(model.status().failed_since, Some(1000));
        let mut compacting = hook(Activity::Working);
        compacting.detail = Some(WorkDetail::CompactingContext);
        observe(
            &mut model,
            compacting,
            Now {
                wall_millis: 2000,
                ..at
            },
        );
        if viewed {
            model.apply(SessionEvent::Viewed, at);
        }
        observe(
            &mut model,
            failed,
            Now {
                wall_millis: 3000,
                ..at
            },
        );
        assert_eq!(
            model.status().failed_since,
            if viewed { None } else { Some(1000) }
        );
        observe(&mut model, hook(Activity::Working), at);
        assert_eq!(model.status().failed_since, None);
    }
}

#[test]
fn rule_8_completion_is_consumed_once_and_failures_do_not_arm_it() {
    let mut model = SessionModel::default();
    let mut compacting = hook(Activity::Working);
    compacting.detail = Some(WorkDetail::CompactingContext);
    observe(&mut model, compacting, now());
    assert!(!model.completion_armed);
    observe(&mut model, hook(Activity::Working), now());
    observe(&mut model, hook(Activity::Ready), now());
    assert!(
        model
            .apply(SessionEvent::TakeCompletion, now())
            .completion_consumed
    );
    model.apply(SessionEvent::Unread { viewed: false }, now());
    assert_eq!(model.status().unread_since, Some(1000));
    assert!(
        !model
            .apply(SessionEvent::TakeCompletion, now())
            .completion_consumed
    );
    for outcome in [TurnOutcome::Interrupted, TurnOutcome::Failed] {
        observe(&mut model, hook(Activity::Working), now());
        let mut ended = hook(Activity::Ready);
        ended.outcome = Some(outcome);
        observe(&mut model, ended, now());
        assert!(
            !model
                .apply(SessionEvent::TakeCompletion, now())
                .completion_consumed
        );
    }
}

#[test]
fn rule_9_delivery_updates_projection_only_at_the_reserved_revision() {
    let mut model = SessionModel::default();
    transition(
        &mut model,
        SessionActivityState::Idle,
        StatusSource::Forwarder,
    );
    let revision = model.revision();
    let status = model.status().clone();
    model.apply(SessionEvent::Delivered { revision }, now());
    assert_eq!(model.activity(), Some(SessionActivityState::Busy));
    assert_eq!(model.status(), &status);
    transition(
        &mut model,
        SessionActivityState::Idle,
        StatusSource::Forwarder,
    );
    model.apply(SessionEvent::Delivered { revision }, now());
    assert_eq!(model.activity(), Some(SessionActivityState::Idle));
}

#[test]
fn rule_10_exit_retains_attention_and_rejects_in_flight_observations() {
    let mut model = SessionModel::default();
    observe(&mut model, hook(Activity::Working), now());
    model.apply(SessionEvent::Unread { viewed: false }, now());
    model.apply(
        SessionEvent::Exited {
            code: Some(1),
            crashed: true,
        },
        now(),
    );
    model.apply(SessionEvent::Detached { stopped: false }, now());
    let status = model.status().clone();
    let effects = model.apply(
        SessionEvent::Observation {
            observation: hook(Activity::Ready),
            live: false,
        },
        now(),
    );
    assert!(effects.publication.is_none());
    assert_eq!(model.status(), &status);
    assert_eq!(model.activity(), None);
    assert!(!model.completion_armed);
    model.apply(SessionEvent::Attached, now());
    assert_eq!(model.status().unread_since, Some(1000));
    assert_eq!(model.status().error_since, None);
}

#[test]
fn status_sources_keep_their_wire_strings() {
    for source in [
        StatusSource::Spawn,
        StatusSource::Fork,
        StatusSource::Resume,
        StatusSource::Forwarder,
        StatusSource::InputSubmit,
        StatusSource::InputInterrupt,
        StatusSource::InputEscape,
        StatusSource::Hook,
        StatusSource::Baseline,
        StatusSource::Unavailable,
    ] {
        let json = serde_json::to_value(source).unwrap();
        assert_eq!(json.as_str(), Some(source.as_str()));
        assert_eq!(
            serde_json::from_value::<StatusSource>(json).unwrap(),
            source
        );
    }
}
