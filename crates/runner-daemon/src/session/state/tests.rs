use super::*;

fn now() -> Now {
    Now {
        monotonic: Instant::now(),
        wall_millis: 1000,
    }
}

#[test]
fn recorded_drafts_keep_delivery_held_after_recent_input_expires() {
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::vte::ansi::Processor;
    use runner_terminal::fixtures::{decode_chunk, Fixture, FixtureEvent, FixtureInput};
    use runner_terminal::input_state::InputTracker;
    use runner_terminal::replay::new_term;

    for name in [
        "input-codex-multiline-paste",
        "input-codex-active-tool-paste",
        "input-codex-coalesced-typing",
    ] {
        let fixture = Fixture::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../runner-terminal/fixtures/{name}.ndjson")),
        )
        .unwrap();
        let start = now();
        let mut term = new_term(fixture.header.cols, fixture.header.rows);
        let mut parser: Processor = Processor::new();
        let mut tracker = InputTracker::new(start.monotonic);
        let mut model = SessionModel::default();
        model.apply(
            SessionEvent::Composer {
                observation: tracker.initial_observation(start.monotonic),
                live: true,
            },
            start,
        );
        for event in fixture.events {
            let (ms, observation) = match event {
                FixtureEvent::Data { ms, data } => {
                    parser.advance(&mut term, &decode_chunk(&data).unwrap());
                    (
                        ms,
                        tracker.observe_output(start.monotonic + Duration::from_millis(ms), &term),
                    )
                }
                FixtureEvent::Input {
                    ms,
                    input: FixtureInput::Event(input),
                } => {
                    use runner_terminal::input_state::InputEvent;
                    use runner_terminal::mappings::InputKind;
                    let class = match &input {
                        InputEvent::Paste { .. }
                        | InputEvent::Key {
                            kind: InputKind::Content { .. },
                        } => Some(LocalInputClass::SetPending),
                        InputEvent::Composing { composing: false } => None,
                        _ => panic!("unexpected input in the unsent draft fixture"),
                    };
                    model.apply(
                        SessionEvent::Input {
                            class,
                            submitted: false,
                        },
                        Now {
                            monotonic: start.monotonic + Duration::from_millis(ms),
                            ..start
                        },
                    );
                    (
                        ms,
                        tracker.observe_input(
                            &input,
                            start.monotonic + Duration::from_millis(ms),
                            &term,
                        ),
                    )
                }
                FixtureEvent::Resize { cols, rows, .. } => {
                    term.resize(TermSize::new(cols as usize, rows as usize));
                    continue;
                }
                _ => panic!("unexpected event in the unsent draft fixture"),
            };
            if let Some(observation) = observation {
                model.apply(
                    SessionEvent::Composer {
                        observation,
                        live: true,
                    },
                    Now {
                        monotonic: start.monotonic + Duration::from_millis(ms),
                        ..start
                    },
                );
            }
        }
        let later = start.monotonic + Duration::from_secs(10);
        assert_eq!(
            model.draft_hold(later),
            Some(crate::router::DeliveryReservation::Drafting {
                composer_visible: true
            }),
            "{name}"
        );
        assert!(!model.draft_quiescent(later), "{name}");
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
        SessionEvent::Agent {
            event: agent::AgentEvent::Published(observation),
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
    model.apply(SessionEvent::BridgeFailed { live: true }, now());
    assert!(!model.completion_armed);
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
fn recovered_identity_clears_agent_state_and_returns_to_baseline() {
    use agent::AgentEvent;
    let mut model = SessionModel::default();
    transition(
        &mut model,
        SessionActivityState::Busy,
        StatusSource::Forwarder,
    );
    for event in [
        AgentEvent::TurnStarted,
        AgentEvent::ToolStarted {
            count: 2,
            question: None,
        },
        AgentEvent::InteractionOpened {
            reason: super::super::status::WaitReason::Answer,
            owners: vec!["old-owner".into()],
        },
    ] {
        model.apply(SessionEvent::Agent { event, live: true }, now());
    }
    assert!(!model.agent.value.interactions.is_empty());
    model.apply(
        SessionEvent::Agent {
            event: AgentEvent::Batch {
                runtime: crate::model::Runtime::Codex,
                events: vec![AgentEvent::ConversationRecovered],
            },
            live: true,
        },
        now(),
    );
    assert_eq!(model.agent.value, agent::TurnState::default());
    assert_eq!(
        model.status().observation.source,
        ObservationSource::Baseline
    );
    assert_eq!(model.status().observation.activity, Activity::Working);
    assert!(model.status().observation.interactions.is_empty());
    assert_eq!(model.status().observation.outcome, None);
    assert!(!model.hook_status_armed);
    assert!(!model.completion_armed);
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

fn editor_draft(model: &mut SessionModel, drafting: bool, at: Now) -> Effects {
    model.apply(
        SessionEvent::Agent {
            event: agent::AgentEvent::EditorDraft { drafting },
            live: true,
        },
        at,
    )
}

#[test]
fn native_draft_overrides_screen_and_clear_releases_once_without_status_change() {
    let mut model = SessionModel::default();
    let at = now();
    observe(&mut model, hook(Activity::Ready), at);
    let status = model.status().clone();
    assert!(!editor_draft(&mut model, false, at).input_cleared);
    model.apply(
        SessionEvent::Input {
            class: Some(LocalInputClass::SetPending),
            submitted: false,
        },
        at,
    );
    assert!(editor_draft(&mut model, true, at).publication.is_none());
    let later = Now {
        monotonic: at.monotonic + Duration::from_secs(3),
        ..at
    };
    model.apply(
        SessionEvent::Composer {
            observation: InputObservation {
                state: InputState::Idle,
                since: later.monotonic,
                composing: false,
                composer_visible: true,
            },
            live: true,
        },
        later,
    );
    assert!(!model.draft_quiescent(later.monotonic));
    assert_eq!(
        model.draft_hold(later.monotonic),
        Some(crate::router::DeliveryReservation::Drafting {
            composer_visible: true
        })
    );
    let cleared = editor_draft(&mut model, false, later);
    assert!(cleared.input_cleared);
    assert!(cleared.publication.is_none());
    assert!(model.draft_quiescent(later.monotonic));
    assert!(!editor_draft(&mut model, false, later).input_cleared);
    assert_eq!(model.status(), &status);
}

#[test]
fn native_submit_clears_only_the_draft_hold_and_preserves_the_working_turn() {
    let mut model = SessionModel::default();
    let at = now();
    observe(&mut model, hook(Activity::Idle), at);
    editor_draft(&mut model, true, at);
    model.apply(
        SessionEvent::Input {
            class: Some(LocalInputClass::ClearPending),
            submitted: true,
        },
        at,
    );
    observe(&mut model, hook(Activity::Working), at);
    let status = model.status().clone();
    let revision = model.revision();
    let cleared = editor_draft(&mut model, false, at);
    assert!(cleared.input_cleared);
    assert!(cleared.publication.is_none());
    assert!(model.draft_quiescent(at.monotonic));
    assert_eq!(model.status(), &status);
    assert_eq!(model.activity(), Some(SessionActivityState::Busy));
    assert_eq!(model.revision(), revision);
    assert!(model.completion_armed());
    observe(&mut model, hook(Activity::Ready), at);
    assert_eq!(model.activity(), Some(SessionActivityState::Idle));
}

#[test]
fn delayed_native_clear_preserves_recent_input_for_a_replacement_draft() {
    let mut model = SessionModel::default();
    let start = now();
    let at = |ms| Now {
        monotonic: start.monotonic + Duration::from_millis(ms),
        ..start
    };
    observe(&mut model, hook(Activity::Idle), start);
    editor_draft(&mut model, true, at(10));
    for ms in [3000, 3050] {
        model.apply(
            SessionEvent::Input {
                class: Some(LocalInputClass::SetPending),
                submitted: false,
            },
            at(ms),
        );
    }
    assert!(editor_draft(&mut model, false, at(3060)).input_cleared);
    assert!(!model.draft_quiescent(at(3061).monotonic));
    assert_eq!(
        model.draft_hold(at(3061).monotonic),
        Some(crate::router::DeliveryReservation::RecentlyTyping(
            Duration::from_millis(1989)
        ))
    );
    editor_draft(&mut model, true, at(3070));
    assert_eq!(
        model.draft_hold(at(6000).monotonic),
        Some(crate::router::DeliveryReservation::Drafting {
            composer_visible: true
        })
    );
}

#[test]
fn native_draft_resets_on_bridge_loss_detach_and_attach_and_ignores_late_events() {
    for reset in [
        SessionEvent::BridgeFailed { live: true },
        SessionEvent::Detached { stopped: true },
        SessionEvent::Attached,
        SessionEvent::Exited {
            code: Some(0),
            crashed: false,
        },
    ] {
        let mut model = SessionModel::default();
        let at = now();
        observe(&mut model, hook(Activity::Idle), at);
        editor_draft(&mut model, true, at);
        assert!(model.native_input.is_some());
        model.apply(reset, at);
        assert!(model.native_input.is_none());
        assert!(model.draft_quiescent(at.monotonic));
        model.apply(
            SessionEvent::Agent {
                event: agent::AgentEvent::EditorDraft { drafting: true },
                live: false,
            },
            at,
        );
        assert!(model.native_input.is_none());
    }
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
        SessionEvent::Agent {
            event: agent::AgentEvent::Published(hook(Activity::Ready)),
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

#[test]
fn codex_native_attention_is_independent_of_hooks_and_local_input() {
    use agent::AgentEvent as A;
    let mut model = SessionModel::default();
    let time = now();
    let hook = |event| SessionEvent::Agent {
        event: A::Batch {
            runtime: crate::model::Runtime::Codex,
            events: vec![event],
        },
        live: true,
    };
    model.apply(SessionEvent::Attached, time);
    model.apply(hook(A::TurnStarted), time);
    let original = model.status().observation.clone();
    let opened = model.apply(
        SessionEvent::CodexAttention {
            blocked: true,
            live: true,
        },
        time,
    );
    assert!(opened.publication.is_some());
    let wait = model.status().observation.interactions[0].clone();
    assert_eq!(wait.reason, WaitReason::Unknown);
    let later = Now {
        wall_millis: time.wall_millis + 90_000,
        ..time
    };
    let duplicate = model.apply(
        SessionEvent::CodexAttention {
            blocked: true,
            live: true,
        },
        later,
    );
    assert!(duplicate.publication.is_none());
    assert_eq!(model.status().observation.interactions, vec![wait.clone()]);
    for event in [
        A::Working {
            detail: Some(WorkDetail::UsingTools),
        },
        A::ToolStarted {
            count: 1,
            question: None,
        },
        A::CompactionStarted,
        A::CompactionEnded,
        A::TurnEnded {
            outcome: TurnOutcome::Completed,
        },
        A::TurnStarted,
        A::TurnEnded {
            outcome: TurnOutcome::Failed,
        },
        A::TurnStarted,
        A::LocalCancel {
            kind: ESCAPE_INTERRUPT,
        },
        A::ToolEnded {
            owner: None,
            count: 0,
            interrupted: false,
            transcript: false,
        },
    ] {
        model.apply(hook(event), later);
        assert_eq!(model.status().observation.interactions, vec![wait.clone()]);
        assert_eq!(model.activity(), Some(SessionActivityState::Busy));
    }
    model.apply(
        SessionEvent::Input {
            class: Some(LocalInputClass::ClearPending),
            submitted: true,
        },
        later,
    );
    assert!(model.status().observation.needs_you());
    let before_clear = model.status().observation.clone();
    let completion = model.completion_armed();
    model.apply(
        SessionEvent::CodexAttention {
            blocked: false,
            live: true,
        },
        later,
    );
    assert!(!model.status().observation.needs_you());
    assert_eq!(model.status().observation.activity, before_clear.activity);
    assert_eq!(model.status().observation.outcome, before_clear.outcome);
    assert_eq!(model.completion_armed(), completion);
    assert_eq!(model.status().unread_since, None);
    model.apply(
        SessionEvent::CodexAttention {
            blocked: true,
            live: true,
        },
        later,
    );
    assert_ne!(model.status().observation.interactions[0].id, wait.id);
    assert_eq!(original.activity, Activity::Working);
}

#[test]
fn codex_native_attention_preserves_fallback_drafts_and_lifecycle_boundaries() {
    for terminal in [
        SessionEvent::Exited {
            code: Some(0),
            crashed: false,
        },
        SessionEvent::Detached { stopped: true },
        SessionEvent::Attached,
    ] {
        let mut model = SessionModel::default();
        let time = now();
        model.apply(
            SessionEvent::Transition {
                state: SessionActivityState::Idle,
                source: StatusSource::Forwarder,
                live: true,
            },
            time,
        );
        model.apply(
            SessionEvent::Input {
                class: Some(LocalInputClass::SetPending),
                submitted: false,
            },
            time,
        );
        model.apply(
            SessionEvent::CodexAttention {
                blocked: true,
                live: false,
            },
            time,
        );
        assert!(
            !model.status().observation.needs_you(),
            "stale launch report"
        );
        model.apply(
            SessionEvent::CodexAttention {
                blocked: true,
                live: true,
            },
            time,
        );
        let wait = model.status().observation.interactions.clone();
        model.apply(SessionEvent::BridgeFailed { live: true }, time);
        model.apply(
            SessionEvent::Transition {
                state: SessionActivityState::Idle,
                source: StatusSource::Forwarder,
                live: true,
            },
            time,
        );
        assert_eq!(model.status().observation.interactions, wait);
        assert_eq!(model.activity(), Some(SessionActivityState::Busy));
        model.apply(
            SessionEvent::CodexAttention {
                blocked: false,
                live: false,
            },
            time,
        );
        assert!(model.status().observation.needs_you());
        model.apply(
            SessionEvent::CodexAttention {
                blocked: false,
                live: true,
            },
            time,
        );
        assert!(model.local_input_pending());
        assert!(model
            .draft_hold(time.monotonic + Duration::from_secs(60))
            .is_some());
        assert_eq!(model.status().observation.outcome, None);
        assert!(!model.completion_armed());
        assert_eq!(model.status().unread_since, None);
        model.apply(
            SessionEvent::CodexAttention {
                blocked: true,
                live: true,
            },
            time,
        );
        model.apply(terminal, time);
        assert!(!model.status().observation.needs_you());
    }
}
