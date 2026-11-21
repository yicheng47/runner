use super::*;

fn now() -> Now {
    Now {
        monotonic: Instant::now(),
        wall_millis: 1000,
    }
}

#[test]
fn observed_draft_cap_uses_the_last_keystroke_and_composition_updates_refresh_it() {
    use crate::router::DeliveryReservation as D;
    let start = now();
    let mut model = SessionModel::default();
    let observation = |composing| InputObservation {
        state: InputState::Drafting,
        since: start.monotonic,
        composing,
        composer_visible: true,
    };
    model.apply(
        SessionEvent::Composer {
            observation: observation(false),
            live: true,
        },
        start,
    );
    assert_eq!(
        model.draft_hold(start.monotonic),
        None,
        "no keystroke since attach"
    );
    model.apply(
        SessionEvent::Input {
            class: Some(LocalInputClass::SetPending),
            submitted: false,
            cancel: false,
        },
        start,
    );
    assert_eq!(
        model.draft_hold(start.monotonic + OBSERVED_DRAFT_HOLD_CAP - Duration::from_millis(1)),
        Some(D::Drafting {
            composer_visible: true
        })
    );
    let boundary = Now {
        monotonic: start.monotonic + OBSERVED_DRAFT_HOLD_CAP,
        ..start
    };
    assert_eq!(model.draft_hold(boundary.monotonic), None);
    model.apply(
        SessionEvent::Composer {
            observation: observation(true),
            live: true,
        },
        boundary,
    );
    assert_eq!(model.last_local_input_at(), Some(boundary.monotonic));
    assert_eq!(
        model.draft_hold(boundary.monotonic),
        Some(D::Drafting {
            composer_visible: true
        })
    );
    let update = Now {
        monotonic: boundary.monotonic + Duration::from_secs(1),
        ..start
    };
    model.apply(
        SessionEvent::Composer {
            observation: observation(true),
            live: true,
        },
        update,
    );
    assert_eq!(model.last_local_input_at(), Some(update.monotonic));
    assert_eq!(
        model.draft_hold(boundary.monotonic + OBSERVED_DRAFT_HOLD_CAP),
        Some(D::Drafting {
            composer_visible: true
        })
    );
}

#[test]
fn native_editor_drafts_and_runners_notice_are_not_capped() {
    use crate::router::DeliveryReservation as D;
    for native in [false, true] {
        let start = now();
        let mut model = SessionModel::default();
        if native {
            editor_draft(&mut model, true, start);
        } else {
            model.apply(
                SessionEvent::Composer {
                    observation: InputObservation {
                        state: InputState::Unsent,
                        since: start.monotonic,
                        composing: false,
                        composer_visible: true,
                    },
                    live: true,
                },
                start,
            );
        }
        assert_eq!(
            model.draft_hold(start.monotonic + Duration::from_secs(26 * 60)),
            Some(if native {
                D::Drafting {
                    composer_visible: true,
                }
            } else {
                D::Unsent
            })
        );
    }
}

#[test]
fn rejected_return_keeps_the_observed_drafts_keystroke_for_the_cap() {
    let start = now();
    let mut model = SessionModel::default();
    let observation = |state| InputObservation {
        state,
        since: start.monotonic,
        composing: false,
        composer_visible: true,
    };
    model.apply(
        SessionEvent::Composer {
            observation: observation(InputState::Submitted),
            live: true,
        },
        start,
    );
    model.apply(
        SessionEvent::Input {
            class: Some(LocalInputClass::ClearPending),
            submitted: true,
            cancel: false,
        },
        start,
    );
    model.apply(
        SessionEvent::Composer {
            observation: observation(InputState::Drafting),
            live: true,
        },
        start,
    );
    assert_eq!(model.last_local_input_at(), Some(start.monotonic));
    assert!(matches!(
        model.draft_hold(start.monotonic + Duration::from_secs(599)),
        Some(crate::router::DeliveryReservation::Drafting { .. })
    ));
    assert_eq!(
        model.draft_hold(start.monotonic + OBSERVED_DRAFT_HOLD_CAP),
        None
    );
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
                            cancel: false,
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
            cancel: false,
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
            cancel: false,
        },
        at,
    );
    assert_eq!(
        effects.publication,
        Some((SessionActivityState::Busy, StatusSource::InputSubmit))
    );
    assert!(effects.input_cleared);
    assert_eq!(model.status().observation.activity, Activity::Working);
    model.apply(SessionEvent::TestInputAt(Some(at.monotonic)), at);
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
            cancel: false,
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
            cancel: false,
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
                cancel: false,
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
        StatusSource::InputTimeout,
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

fn codex_update(
    state: runner_terminal::program_status::State,
    kind: Option<&str>,
) -> Box<runner_terminal::program_status::Update> {
    use runner_terminal::program_status::{Report, State, Update};
    let report = Report {
        state,
        id: String::new(),
        kind: kind.map(str::to_owned),
        progress: None,
        app: (state != State::Clear).then(|| "codex".into()),
        title: None,
        message: None,
    };
    Box::new(Update {
        current: (state != State::Clear).then(|| report.clone()),
        root: (state != State::Clear).then(|| report.clone()),
        active_children: Vec::new(),
        trigger: Some(report),
    })
}

#[test]
fn codex_native_attention_is_independent_of_hooks_and_local_input() {
    use agent::AgentEvent as A;
    use runner_terminal::program_status::State as P;
    let mut model = SessionModel::default();
    model.set_native_attention(true);
    let time = now();
    let hook = |event| SessionEvent::Agent {
        event: A::Batch {
            runtime: crate::model::Runtime::Codex,
            events: vec![event],
        },
        live: true,
    };
    let mut seq = 0u64;
    let mut program = |model: &mut SessionModel,
                       update: Box<runner_terminal::program_status::Update>,
                       at: Now| {
        seq += 1;
        model.apply(
            SessionEvent::ProgramStatus {
                seq,
                subseq: 0,
                update,
                live: true,
            },
            at,
        )
    };
    model.apply(SessionEvent::Attached, time);
    model.apply(hook(A::TurnStarted), time);
    let original = model.status().observation.clone();

    // Every blocked kind maps to the expected WaitReason and resolves on working/idle/done/error/clear.
    for (kind, expected_reason, resolver) in [
        (None, WaitReason::Unknown, codex_update(P::Working, None)),
        (
            Some("permission"),
            WaitReason::Approval,
            codex_update(P::Idle, None),
        ),
        (
            Some("question"),
            WaitReason::Answer,
            codex_update(P::Done, None),
        ),
        (
            Some("auth"),
            WaitReason::Unknown,
            codex_update(P::Error, None),
        ),
        (
            Some("permission"),
            WaitReason::Approval,
            codex_update(P::Clear, None),
        ),
    ] {
        let opened = program(&mut model, codex_update(P::Blocked, kind), time);
        assert_eq!(
            opened.publication,
            Some((SessionActivityState::Busy, StatusSource::CodexAttention))
        );
        let wait = model.status().observation.interactions[0].clone();
        assert_eq!(wait.reason, expected_reason, "{kind:?}");
        assert_eq!(model.status().observation.source, ObservationSource::Hook);
        assert!(model.status().observation.program_status.is_none());

        let later = Now {
            wall_millis: time.wall_millis + 90_000,
            ..time
        };
        let duplicate = program(&mut model, codex_update(P::Blocked, kind), later);
        assert!(duplicate.publication.is_none());
        assert_eq!(model.status().observation.interactions, vec![wait.clone()]);

        let before_clear = model.status().observation.clone();
        let completion = model.completion_armed();
        let cleared = program(&mut model, resolver, later);
        assert_eq!(
            cleared.publication,
            Some((SessionActivityState::Busy, StatusSource::CodexAttention))
        );
        assert!(cleared.input_cleared);
        assert!(!model.status().observation.needs_you());
        assert_eq!(model.status().observation.activity, before_clear.activity);
        assert_eq!(model.status().observation.outcome, before_clear.outcome);
        assert_eq!(model.completion_armed(), completion);
        assert_eq!(model.status().unread_since, None);
    }

    // Changing kind while blocked replaces the wait with the new reason and id.
    let opened = program(
        &mut model,
        codex_update(P::Blocked, Some("permission")),
        time,
    );
    assert!(opened.publication.is_some());
    let approval_wait = model.status().observation.interactions[0].clone();
    assert_eq!(approval_wait.reason, WaitReason::Approval);
    let later = Now {
        wall_millis: time.wall_millis + 90_000,
        ..time
    };
    let switched = program(
        &mut model,
        codex_update(P::Blocked, Some("question")),
        later,
    );
    assert!(switched.publication.is_some());
    let wait = model.status().observation.interactions[0].clone();
    assert_eq!(wait.reason, WaitReason::Answer);
    assert_ne!(wait.id, approval_wait.id);

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
            cancel: false,
        },
        later,
    );
    assert!(model.status().observation.needs_you());
    let before_clear = model.status().observation.clone();
    let completion = model.completion_armed();
    program(&mut model, codex_update(P::Done, None), later);
    assert!(!model.status().observation.needs_you());
    assert_eq!(model.status().observation.activity, before_clear.activity);
    assert_eq!(model.status().observation.outcome, before_clear.outcome);
    assert_eq!(model.completion_armed(), completion);
    assert_eq!(model.status().unread_since, None);

    // Non-blocked reports while hooks are armed and no wait is open are no-ops.
    for update in [
        codex_update(P::Working, None),
        codex_update(P::Idle, None),
        codex_update(P::Done, None),
        codex_update(P::Error, None),
        codex_update(P::Clear, None),
    ] {
        assert!(program(&mut model, update, later).publication.is_none());
    }

    program(&mut model, codex_update(P::Blocked, None), later);
    assert_ne!(model.status().observation.interactions[0].id, wait.id);
    assert_eq!(original.activity, Activity::Working);
}

#[test]
fn codex_native_attention_preserves_fallback_drafts_and_lifecycle_boundaries() {
    use runner_terminal::program_status::State as P;
    for terminal in [
        SessionEvent::Exited {
            code: Some(0),
            crashed: false,
        },
        SessionEvent::Detached { stopped: true },
        SessionEvent::Attached,
    ] {
        let mut model = SessionModel::default();
        model.set_native_attention(true);
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
                cancel: false,
            },
            time,
        );
        model.apply(
            SessionEvent::ProgramStatus {
                seq: 1,
                subseq: 0,
                update: codex_update(P::Blocked, Some("permission")),
                live: false,
            },
            time,
        );
        assert!(
            !model.status().observation.needs_you(),
            "stale launch report"
        );
        model.apply(
            SessionEvent::ProgramStatus {
                seq: 1,
                subseq: 0,
                update: codex_update(P::Blocked, Some("permission")),
                live: true,
            },
            time,
        );
        let wait = model.status().observation.interactions.clone();
        assert_eq!(wait[0].reason, WaitReason::Approval);
        assert!(model.status().observation.program_status.is_none());
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
            SessionEvent::ProgramStatus {
                seq: 2,
                subseq: 0,
                update: codex_update(P::Clear, None),
                live: false,
            },
            time,
        );
        assert!(model.status().observation.needs_you());
        model.apply(
            SessionEvent::ProgramStatus {
                seq: 2,
                subseq: 0,
                update: codex_update(P::Clear, None),
                live: true,
            },
            time,
        );
        assert!(!model.status().observation.needs_you());
        assert_eq!(model.status().observation.activity, Activity::Idle);
        assert_eq!(
            model.status().observation.source,
            ObservationSource::Baseline
        );
        assert!(model.status().observation.program_status.is_none());
        assert!(model.local_input_pending());
        assert!(model
            .draft_hold(time.monotonic + Duration::from_secs(60))
            .is_some());
        assert_eq!(model.status().observation.outcome, None);
        assert!(!model.completion_armed());
        assert_eq!(model.status().unread_since, None);
        model.apply(
            SessionEvent::ProgramStatus {
                seq: 3,
                subseq: 0,
                update: codex_update(P::Blocked, Some("question")),
                live: true,
            },
            time,
        );
        model.apply(terminal, time);
        assert!(!model.status().observation.needs_you());
    }
}

#[test]
fn codex_pre_hook_blocked_wait_survives_hook_takeover_and_clears_on_report() {
    use agent::AgentEvent as A;
    use runner_terminal::program_status::State as P;
    let mut model = SessionModel::default();
    model.set_native_attention(true);
    let time = now();
    model.apply(SessionEvent::Attached, time);
    model.apply(
        SessionEvent::ProgramStatus {
            seq: 1,
            subseq: 0,
            update: codex_update(P::Blocked, Some("permission")),
            live: true,
        },
        time,
    );
    let wait = model.status().observation.interactions[0].clone();
    assert_eq!(wait.reason, WaitReason::Approval);
    assert!(model.status().observation.program_status.is_none());
    model.apply(
        SessionEvent::Agent {
            event: A::Batch {
                runtime: crate::model::Runtime::Codex,
                events: vec![A::TurnStarted],
            },
            live: true,
        },
        time,
    );
    assert_eq!(model.status().observation.source, ObservationSource::Hook);
    assert!(model.status().observation.program_status.is_none());
    assert_eq!(model.status().observation.interactions, vec![wait]);
    let cleared = model.apply(
        SessionEvent::ProgramStatus {
            seq: 2,
            subseq: 0,
            update: codex_update(P::Working, None),
            live: true,
        },
        time,
    );
    assert_eq!(
        cleared.publication,
        Some((SessionActivityState::Busy, StatusSource::CodexAttention))
    );
    assert!(!model.status().observation.needs_you());
    assert_eq!(model.status().observation.activity, Activity::Working);
}

#[test]
fn codex_overlay_filters_foreign_reports_and_retains_baseline_completion_without_hooks() {
    use runner_terminal::program_status::{Report, State as P, Update};
    let mut model = SessionModel::default();
    model.set_native_attention(true);
    let time = now();
    let mut seq = 0u64;
    let mut send = |model: &mut SessionModel, update: Box<Update>| {
        seq += 1;
        model.apply(
            SessionEvent::ProgramStatus {
                seq,
                subseq: 0,
                update,
                live: true,
            },
            time,
        )
    };
    let report = |state: P, id: &str, app: Option<&str>, kind: Option<&str>| Report {
        state,
        id: id.into(),
        kind: kind.map(str::to_owned),
        progress: None,
        app: app.map(str::to_owned),
        title: None,
        message: None,
    };

    // Baseline Busy + armed completion before any hook arrives.
    transition(
        &mut model,
        SessionActivityState::Busy,
        StatusSource::Forwarder,
    );
    model.apply(SessionEvent::ArmCompletion, time);

    // Foreign root or child blocked reports never open Codex's overlay.
    for foreign in [
        report(P::Blocked, "", Some("other"), Some("permission")),
        report(P::Blocked, "", None, Some("permission")),
        report(P::Blocked, "sub", Some("codex"), Some("question")),
        report(P::Blocked, "sub", Some("other"), Some("question")),
    ] {
        let effects = send(
            &mut model,
            Box::new(Update {
                current: Some(foreign.clone()),
                root: foreign.id.is_empty().then(|| foreign.clone()),
                active_children: (!foreign.id.is_empty()
                    && matches!(foreign.state, P::Working | P::Blocked))
                .then(|| runner_terminal::program_status::ChildCount {
                    app: foreign.app.clone(),
                    active: 1,
                })
                .into_iter()
                .collect(),
                trigger: Some(foreign),
            }),
        );
        assert!(effects.publication.is_none());
        assert!(!model.status().observation.needs_you());
        assert!(model.status().observation.program_status.is_none());
    }

    // Codex working without a wait does not enter full ProgramStatus or suppress baseline Idle.
    assert!(send(&mut model, codex_update(P::Working, None))
        .publication
        .is_none());
    assert!(model.status().observation.program_status.is_none());
    assert_eq!(
        transition(
            &mut model,
            SessionActivityState::Idle,
            StatusSource::Forwarder
        )
        .publication,
        Some((SessionActivityState::Idle, StatusSource::Forwarder))
    );
    assert!(model.completion_armed());

    // Open a real Codex permission wait, then interleave foreign root/child reports.
    assert_eq!(
        send(&mut model, codex_update(P::Blocked, Some("permission"))).publication,
        Some((SessionActivityState::Busy, StatusSource::CodexAttention))
    );
    let wait = model.status().observation.interactions[0].clone();
    assert_eq!(wait.reason, WaitReason::Approval);

    let foreign_child = report(P::Blocked, "sub", Some("other"), Some("question"));
    for trigger in [
        report(P::Idle, "", Some("other"), None),
        report(P::Done, "", Some("other"), None),
        report(P::Clear, "", Some("other"), None),
        report(P::Clear, "sub", None, None),
        foreign_child.clone(),
        report(P::Blocked, "sub", Some("codex"), Some("question")),
    ] {
        let effects = send(
            &mut model,
            Box::new(Update {
                current: Some(trigger.clone()),
                root: trigger.id.is_empty().then(|| trigger.clone()),
                active_children: (!trigger.id.is_empty()
                    && matches!(trigger.state, P::Working | P::Blocked))
                .then(|| runner_terminal::program_status::ChildCount {
                    app: trigger.app.clone(),
                    active: 1,
                })
                .into_iter()
                .collect(),
                trigger: Some(trigger),
            }),
        );
        assert!(effects.publication.is_none());
        assert_eq!(model.status().observation.interactions, vec![wait.clone()]);
    }

    // Even when a foreign blocked child wins aggregate `current`, Codex's own root Done
    // trigger releases the Codex wait and restores baseline Idle without disarming completion.
    let released = send(
        &mut model,
        Box::new(Update {
            current: Some(foreign_child.clone()),
            root: None,
            active_children: vec![runner_terminal::program_status::ChildCount {
                app: foreign_child.app.clone(),
                active: 1,
            }],
            trigger: Some(report(P::Done, "", Some("codex"), None)),
        }),
    );
    assert_eq!(
        released.publication,
        Some((SessionActivityState::Idle, StatusSource::CodexAttention))
    );
    assert!(released.input_cleared);
    assert!(!model.status().observation.needs_you());
    assert_eq!(model.status().observation.activity, Activity::Idle);
    assert_eq!(
        model.status().observation.source,
        ObservationSource::Baseline
    );
    assert!(model.status().observation.program_status.is_none());
    assert!(model.completion_armed());
}

fn program(
    model: &mut SessionModel,
    seq: u64,
    state: Option<runner_terminal::program_status::State>,
) -> Effects {
    let report = state.map(|state| runner_terminal::program_status::Report {
        state,
        id: String::new(),
        kind: Some("permission".into()),
        progress: None,
        app: Some("test".into()),
        title: None,
        message: Some("Approve this?".into()),
    });
    model.apply(
        SessionEvent::ProgramStatus {
            seq,
            subseq: 0,
            update: Box::new(runner_terminal::program_status::Update {
                current: report.clone(),
                root: report.clone(),
                active_children: Vec::new(),
                trigger: report,
            }),
            live: true,
        },
        now(),
    )
}

#[test]
fn program_status_maps_work_waits_and_outcomes_without_completion_attention() {
    use runner_terminal::program_status::State as P;
    let mut model = SessionModel::default();
    for (seq, state, activity, outcome) in [
        (1, P::Working, Activity::Working, None),
        (2, P::Blocked, Activity::Idle, None),
        (3, P::Idle, Activity::Idle, None),
        (4, P::Done, Activity::Idle, Some(TurnOutcome::Completed)),
        (5, P::Error, Activity::Idle, Some(TurnOutcome::Failed)),
    ] {
        let effects = program(&mut model, seq, Some(state));
        assert_eq!(effects.publication.unwrap().1, StatusSource::ProgramStatus);
        assert_eq!(model.status().observation.activity, activity);
        assert_eq!(model.status().observation.outcome, outcome);
        assert_eq!(model.status().observation.needs_you(), state == P::Blocked);
        assert_eq!(model.status().unread_since, None);
        assert!(!model.completion_armed());
        assert_eq!(
            model
                .status()
                .observation
                .program_status
                .as_ref()
                .unwrap()
                .message
                .as_deref(),
            Some("Approve this?")
        );
    }
    model.apply(SessionEvent::ArmCompletion, now());
    program(&mut model, 6, Some(P::Working));
    program(&mut model, 7, None);
    assert!(model.completion_armed());
    assert!(model.status().observation.program_status.is_none());
}

#[test]
fn authoritative_program_status_arms_completion_on_work_and_disarms_on_interrupt_error_or_clear() {
    use runner_terminal::program_status::{Report, State as P, Update};
    let mut model = SessionModel::default();
    model.set_program_status_authority(true);

    let emit = |model: &mut SessionModel,
                seq: u64,
                state: Option<P>,
                kind: Option<&str>,
                msg: Option<&str>| {
        let report = state.map(|state| Report {
            state,
            id: String::new(),
            kind: kind.map(str::to_owned),
            progress: None,
            app: Some("agy".into()),
            title: None,
            message: msg.map(str::to_owned),
        });
        model.apply(
            SessionEvent::ProgramStatus {
                seq,
                subseq: 0,
                update: Box::new(Update {
                    current: report.clone(),
                    root: report.clone(),
                    active_children: Vec::new(),
                    trigger: report,
                }),
                live: true,
            },
            now(),
        )
    };

    // Pre-turn auth/trust Blocked -> Working counts as starting a turn once; mid-turn Blocked -> Working does not re-increment.
    let pre_wait = emit(
        &mut model,
        1,
        Some(P::Blocked),
        Some("auth"),
        Some("Sign in"),
    );
    assert!(!pre_wait.program_turn_started);
    let started = emit(&mut model, 2, Some(P::Working), None, None);
    assert!(started.program_turn_started);
    assert!(model.completion_armed());
    assert_eq!(model.activity(), Some(SessionActivityState::Busy));

    for (seq, kind, expected_reason, msg) in [
        (3, "permission", WaitReason::Approval, "Allow bash?"),
        (4, "question", WaitReason::Answer, "Which branch?"),
        (5, "auth", WaitReason::Unknown, "Sign in required"),
    ] {
        let wait = emit(&mut model, seq, Some(P::Blocked), Some(kind), Some(msg));
        assert!(!wait.program_turn_started);
        assert!(model.completion_armed(), "{kind} wait preserves completion");
        assert!(model.status().observation.needs_you());
        assert_eq!(
            model.status().observation.interactions[0].reason,
            expected_reason
        );
        assert_eq!(
            model
                .status()
                .observation
                .program_status
                .as_ref()
                .unwrap()
                .message
                .as_deref(),
            Some(msg)
        );
    }

    let resumed = emit(&mut model, 6, Some(P::Working), None, None);
    assert!(
        !resumed.program_turn_started,
        "mid-turn Blocked -> Working must not count a second turn"
    );
    assert!(model.completion_armed());
    assert!(!model.status().observation.needs_you());

    let done = emit(&mut model, 7, Some(P::Done), None, None);
    assert_eq!(
        done.publication,
        Some((SessionActivityState::Idle, StatusSource::ProgramStatus))
    );
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Completed)
    );
    assert!(model.completion_armed());
    assert!(
        model
            .apply(SessionEvent::TakeCompletion, now())
            .completion_consumed
    );
    assert!(!model.completion_armed());

    // Working -> Idle (Esc interrupt) disarms completion without TurnOutcome::Interrupted or Failed.
    assert!(emit(&mut model, 8, Some(P::Working), None, None).program_turn_started);
    assert!(model.completion_armed());
    emit(&mut model, 9, Some(P::Idle), None, None);
    assert!(!model.completion_armed());
    assert_eq!(model.status().observation.activity, Activity::Idle);
    assert_eq!(model.status().observation.outcome, None);
    assert_eq!(model.status().failed_since, None);

    // Working -> Error disarms completion and records Failed.
    assert!(emit(&mut model, 10, Some(P::Working), None, None).program_turn_started);
    assert!(model.completion_armed());
    emit(&mut model, 11, Some(P::Error), None, Some("Quota exceeded"));
    assert!(!model.completion_armed());
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Failed)
    );
    assert!(model.status().failed_since.is_some());

    // Working -> clear disarms completion under both Idle and Busy output baselines,
    // so a subsequent Forwarder Idle never fabricates an unread completion without Done.
    for baseline in [SessionActivityState::Idle, SessionActivityState::Busy] {
        let mut cleared = SessionModel::default();
        cleared.set_program_status_authority(true);
        transition(&mut cleared, baseline, StatusSource::Forwarder);
        emit(&mut cleared, 1, Some(P::Working), None, None);
        assert!(cleared.completion_armed());
        emit(&mut cleared, 2, None, None, None);
        assert!(
            !cleared.completion_armed(),
            "clear must leave completion disarmed under {baseline:?} baseline"
        );
        assert!(cleared.status().observation.program_status.is_none());
        assert!(
            !cleared
                .apply(SessionEvent::TakeCompletion, now())
                .completion_consumed
        );
        transition(
            &mut cleared,
            SessionActivityState::Idle,
            StatusSource::Forwarder,
        );
        assert!(!cleared.completion_armed());
        assert!(
            !cleared
                .apply(SessionEvent::TakeCompletion, now())
                .completion_consumed
        );
    }

    // Multiple reports sharing one chunk seq (increasing subseq) are accepted in order,
    // while stale or duplicate (seq, subseq) pairs are ignored.
    let mut coalesced = SessionModel::default();
    coalesced.set_program_status_authority(true);
    let agy_update = |state| {
        let report = Report {
            state,
            id: String::new(),
            kind: None,
            progress: None,
            app: Some("agy".into()),
            title: None,
            message: None,
        };
        Box::new(Update {
            current: Some(report.clone()),
            root: Some(report.clone()),
            active_children: Vec::new(),
            trigger: Some(report),
        })
    };
    let first = coalesced.apply(
        SessionEvent::ProgramStatus {
            seq: 5,
            subseq: 0,
            update: agy_update(P::Working),
            live: true,
        },
        now(),
    );
    assert!(first.program_turn_started);
    assert!(coalesced.completion_armed());
    let second = coalesced.apply(
        SessionEvent::ProgramStatus {
            seq: 5,
            subseq: 1,
            update: agy_update(P::Done),
            live: true,
        },
        now(),
    );
    assert_eq!(
        second.publication,
        Some((SessionActivityState::Idle, StatusSource::ProgramStatus))
    );
    assert!(coalesced.completion_armed());
    let stale = coalesced.apply(
        SessionEvent::ProgramStatus {
            seq: 5,
            subseq: 0,
            update: agy_update(P::Idle),
            live: true,
        },
        now(),
    );
    assert!(stale.publication.is_none());
    assert!(coalesced.completion_armed());
}

#[test]
fn program_status_wait_identity_survives_repeats_and_release_preserves_draft() {
    use runner_terminal::program_status::State as P;
    let mut model = SessionModel::default();
    program(&mut model, 1, Some(P::Blocked));
    let wait = model.status().observation.interactions[0].clone();
    assert_eq!(wait.reason, WaitReason::Approval);
    program(&mut model, 2, Some(P::Blocked));
    assert_eq!(model.status().observation.interactions, vec![wait]);
    model.apply(SessionEvent::TestPending(true), now());
    let effects = program(&mut model, 3, Some(P::Working));
    assert!(effects.input_cleared);
    assert!(model.local_input_pending());
    assert!(model.draft_hold(now().monotonic).is_some());
}

#[test]
fn hooks_override_program_status_until_bridge_failure_and_a_fresh_report() {
    use runner_terminal::program_status::State as P;
    let mut model = SessionModel::default();
    program(&mut model, 1, Some(P::Blocked));
    model.apply(
        SessionEvent::Agent {
            event: agent::AgentEvent::TurnStarted,
            live: true,
        },
        now(),
    );
    let hook = model.status().clone();
    assert_eq!(hook.observation.source, ObservationSource::Hook);
    assert!(program(&mut model, 2, Some(P::Idle)).publication.is_none());
    assert_eq!(model.status(), &hook);
    model.apply(SessionEvent::BridgeFailed { live: true }, now());
    assert!(model.status().observation.program_status.is_none());
    assert!(program(&mut model, 1, Some(P::Blocked))
        .publication
        .is_none());
    program(&mut model, 3, Some(P::Idle));
    assert!(model.status().observation.program_status.is_some());
    let exact = model.status().clone();
    assert!(transition(
        &mut model,
        SessionActivityState::Busy,
        StatusSource::Forwarder
    )
    .publication
    .is_none());
    assert_eq!(model.status(), &exact);
}

#[test]
fn process_exit_clears_program_status_and_rejects_late_reports() {
    use runner_terminal::program_status::State as P;
    let mut model = SessionModel::default();
    program(&mut model, 1, Some(P::Blocked));
    model.apply(
        SessionEvent::Exited {
            code: Some(0),
            crashed: false,
        },
        now(),
    );
    assert!(model.status().observation.program_status.is_none());
    assert!(!model.status().observation.needs_you());
    assert!(program(&mut model, 2, Some(P::Working))
        .publication
        .is_none());
    assert_eq!(model.status().lifecycle, Lifecycle::Stopped);
    model.apply(SessionEvent::Attached, now());
    assert!(program(&mut model, 1, Some(P::Working))
        .publication
        .is_some());
}

fn later(at: Now, window: Duration) -> Now {
    Now {
        monotonic: at.monotonic + window,
        wall_millis: at.wall_millis,
    }
}

fn submit(model: &mut SessionModel, at: Now) -> Effects {
    model.apply(
        SessionEvent::Input {
            class: Some(LocalInputClass::ClearPending),
            submitted: true,
            cancel: false,
        },
        at,
    )
}

fn expire(model: &mut SessionModel, at: Now) -> Effects {
    model.apply(SessionEvent::SubmitTimeout { live: true }, at)
}

fn hook_idle(at: Now) -> SessionModel {
    let mut model = SessionModel::default();
    observe(&mut model, hook(Activity::Ready), at);
    assert_eq!(model.activity(), Some(SessionActivityState::Idle));
    model
}

const IDLE: Option<SessionActivityState> = Some(SessionActivityState::Idle);
const BUSY: Option<SessionActivityState> = Some(SessionActivityState::Busy);

#[test]
fn a_submit_that_starts_no_turn_returns_to_the_last_hook_state_after_the_window() {
    let start = now();
    let mut model = hook_idle(start);
    let hooked = model.status().clone();
    assert!(!model.completion_armed());
    assert_eq!(
        submit(&mut model, start).publication,
        Some((SessionActivityState::Busy, StatusSource::InputSubmit))
    );
    assert!(model.unconfirmed_busy());
    assert!(model.completion_armed());

    let almost = later(start, SUBMIT_TURN_START_WINDOW - Duration::from_secs(1));
    assert!(!model.submit_busy_due(almost.monotonic));
    assert!(expire(&mut model, almost).publication.is_none());
    assert_eq!(model.activity(), BUSY);

    let due = later(start, SUBMIT_TURN_START_WINDOW);
    assert!(model.submit_busy_due(due.monotonic));
    let revision = model.revision();
    assert_eq!(
        expire(&mut model, due).publication,
        Some((SessionActivityState::Idle, StatusSource::InputTimeout))
    );
    assert_eq!(model.activity(), IDLE);
    assert_ne!(model.revision(), revision);
    assert_eq!(model.status(), &hooked, "the hook's state is what remains");
    assert!(!model.unconfirmed_busy());
    assert!(
        !model.completion_armed(),
        "no turn ran, so none may complete"
    );

    assert!(!model.submit_busy_due(due.monotonic));
    assert!(expire(&mut model, due).publication.is_none());
}

#[test]
fn a_turn_that_starts_in_time_stays_busy_and_a_late_one_restores_busy() {
    let start = now();
    let due = later(start, SUBMIT_TURN_START_WINDOW);

    let mut model = hook_idle(start);
    submit(&mut model, start);
    let working = observe(
        &mut model,
        hook(Activity::Working),
        later(start, Duration::from_secs(2)),
    );
    assert_eq!(
        working.publication,
        Some((SessionActivityState::Busy, StatusSource::Hook))
    );
    assert!(
        !model.unconfirmed_busy(),
        "a reported turn confirms the busy"
    );
    assert!(expire(&mut model, due).publication.is_none());
    assert_eq!(model.activity(), BUSY);
    observe(
        &mut model,
        hook(Activity::Ready),
        later(due, Duration::from_secs(60)),
    );
    assert_eq!(model.activity(), IDLE);
    assert!(
        expire(&mut model, later(due, Duration::from_secs(120)))
            .publication
            .is_none(),
        "a finished turn is not reverted again"
    );
    assert_eq!(model.activity(), IDLE);

    let mut model = hook_idle(start);
    submit(&mut model, start);
    assert!(expire(&mut model, due).publication.is_some());
    let late = observe(
        &mut model,
        hook(Activity::Working),
        later(due, Duration::from_secs(4)),
    );
    assert_eq!(
        late.publication,
        Some((SessionActivityState::Busy, StatusSource::Hook))
    );
    assert_eq!(model.activity(), BUSY);
    assert!(model.completion_armed(), "the late turn completes normally");
}

#[test]
fn output_proves_a_turn_only_to_sessions_with_no_other_status() {
    let start = now();
    let due = later(start, SUBMIT_TURN_START_WINDOW);
    let silent = |model: &mut SessionModel| {
        submit(model, start);
        expire(model, due)
    };

    let baseline = || {
        let mut model = SessionModel::default();
        transition(
            &mut model,
            SessionActivityState::Idle,
            StatusSource::Forwarder,
        );
        model
    };
    let mut model = baseline();
    let before = model.status().clone();
    let effects = silent(&mut model);
    assert_eq!(
        effects.publication,
        Some((SessionActivityState::Idle, StatusSource::InputTimeout))
    );
    assert_eq!(
        model.status(),
        &before,
        "the submit's estimate is withdrawn"
    );

    let mut model = baseline();
    submit(&mut model, start);
    model.apply(SessionEvent::Output, later(start, Duration::from_secs(1)));
    assert!(
        expire(&mut model, due).publication.is_none(),
        "the idle detector owns the way back once output has flowed"
    );
    assert_eq!(model.activity(), BUSY);
    assert_eq!(model.status().observation.activity, Activity::Working);

    // Codex redraws its composer on an Enter it ignores, so output means nothing under hooks.
    let mut model = hook_idle(start);
    submit(&mut model, start);
    model.apply(SessionEvent::Output, later(start, Duration::from_secs(1)));
    assert!(expire(&mut model, due).publication.is_some());
    assert_eq!(model.activity(), IDLE);

    use runner_terminal::program_status::State as P;
    let mut model = SessionModel::default();
    transition(
        &mut model,
        SessionActivityState::Idle,
        StatusSource::Forwarder,
    );
    program(&mut model, 1, Some(P::Done));
    assert_eq!(model.activity(), IDLE);
    submit(&mut model, start);
    model.apply(SessionEvent::Output, later(start, Duration::from_secs(1)));
    assert!(expire(&mut model, due).publication.is_some());
    assert_eq!(model.activity(), IDLE);
}

#[test]
fn a_confirmed_delivery_without_a_turn_reverts_but_never_at_a_stale_revision() {
    let start = now();
    let due = later(start, SUBMIT_TURN_START_WINDOW);
    let mut model = hook_idle(start);
    let hooked = model.status().clone();
    let stale = model.revision();
    submit(&mut model, start);
    expire(&mut model, due);
    model.apply(SessionEvent::Delivered { revision: stale }, due);
    assert_eq!(model.activity(), IDLE, "an older reservation does not land");
    assert!(!model.submit_busy_due(later(due, SUBMIT_TURN_START_WINDOW).monotonic));

    let revision = model.revision();
    model.apply(SessionEvent::Delivered { revision }, due);
    assert_eq!(model.activity(), BUSY);
    assert!(model.unconfirmed_busy());
    assert_eq!(model.status(), &hooked);
    assert!(expire(&mut model, later(due, SUBMIT_TURN_START_WINDOW))
        .publication
        .is_some());
    assert_eq!(model.activity(), IDLE);
    assert_eq!(model.status(), &hooked);

    let mut unknown = SessionModel::default();
    let revision = unknown.revision();
    unknown.apply(SessionEvent::Delivered { revision }, start);
    assert_eq!(unknown.activity(), BUSY);
    assert!(
        expire(&mut unknown, due).publication.is_none(),
        "an unknown state has no idle to return to"
    );
}

#[test]
fn a_failed_write_withdraws_the_busy_it_armed() {
    let start = now();
    let mut model = hook_idle(start);
    let before = model.status().clone();
    let checkpoint = model.input_checkpoint();
    submit(&mut model, start);
    let after = model.input_checkpoint();
    model.apply(
        SessionEvent::InputWriteFailed {
            before: Box::new(checkpoint),
            after: Box::new(after),
        },
        start,
    );
    assert_eq!(model.activity(), IDLE);
    assert_eq!(model.status(), &before);
    assert!(!model.submit_busy_due(later(start, SUBMIT_TURN_START_WINDOW).monotonic));
    assert!(expire(&mut model, later(start, SUBMIT_TURN_START_WINDOW))
        .publication
        .is_none());
}

#[test]
fn detach_attach_and_exit_drop_the_armed_busy() {
    let start = now();
    let due = later(start, SUBMIT_TURN_START_WINDOW);
    for (name, event) in [
        ("detached", SessionEvent::Detached { stopped: false }),
        ("attached", SessionEvent::Attached),
        (
            "exited",
            SessionEvent::Exited {
                code: Some(0),
                crashed: false,
            },
        ),
    ] {
        let mut model = hook_idle(start);
        submit(&mut model, start);
        model.apply(event, start);
        assert!(!model.submit_armed(), "{name}");
        assert!(expire(&mut model, due).publication.is_none(), "{name}");
    }

    let mut model = hook_idle(start);
    submit(&mut model, start);
    assert!(
        model
            .apply(SessionEvent::SubmitTimeout { live: false }, due)
            .publication
            .is_none(),
        "a dead launch publishes nothing"
    );
    assert!(!model.submit_armed());
}

#[test]
fn a_second_return_restarts_the_wait_while_no_turn_has_shown() {
    let start = now();
    let second = later(start, Duration::from_millis(9_900));
    let first_due = later(start, SUBMIT_TURN_START_WINDOW);
    let second_due = later(second, SUBMIT_TURN_START_WINDOW);

    // The genuine prompt's hook lands inside the second window, after the first one ran out.
    let mut model = hook_idle(start);
    let hooked = model.status().clone();
    submit(&mut model, start);
    assert!(
        submit(&mut model, second).publication.is_none(),
        "the Busy is already published"
    );
    assert!(!model.submit_busy_due(first_due.monotonic));
    assert!(expire(&mut model, first_due).publication.is_none());
    assert_eq!(model.activity(), BUSY);
    let working = observe(
        &mut model,
        hook(Activity::Working),
        later(first_due, Duration::from_secs(5)),
    );
    assert_eq!(
        working.publication,
        Some((SessionActivityState::Busy, StatusSource::Hook))
    );
    assert!(expire(&mut model, second_due).publication.is_none());
    assert_eq!(model.activity(), BUSY);

    // With no turn the new deadline reverts to the baseline of the first submit.
    let mut model = hook_idle(start);
    submit(&mut model, start);
    submit(&mut model, second);
    assert!(model.submit_busy_due(second_due.monotonic));
    assert_eq!(
        expire(&mut model, second_due).publication,
        Some((SessionActivityState::Idle, StatusSource::InputTimeout))
    );
    assert_eq!(model.status(), &hooked);
    assert!(
        !model.completion_armed(),
        "the baseline is the first submit's, not the second's"
    );

    // A Busy the hook confirmed has no armed submit, so a Return in it arms nothing.
    let mut model = hook_idle(start);
    submit(&mut model, start);
    observe(
        &mut model,
        hook(Activity::Working),
        later(start, Duration::from_secs(1)),
    );
    submit(&mut model, later(start, Duration::from_secs(2)));
    assert!(!model.submit_busy_due(later(start, Duration::from_secs(600)).monotonic));
    assert!(expire(&mut model, later(start, Duration::from_secs(600)))
        .publication
        .is_none());
    assert_eq!(model.activity(), BUSY);
}

#[test]
fn a_failed_write_withdraws_only_what_its_input_still_holds() {
    let start = now();
    let typed = |model: &mut SessionModel, class, submitted, at| {
        let before = model.input_checkpoint();
        model.apply(
            SessionEvent::Input {
                class,
                submitted,
                cancel: false,
            },
            at,
        );
        (before, model.input_checkpoint())
    };

    // Viewing the session while the write waits stays viewed.
    let mut model = hook_idle(start);
    model.apply(SessionEvent::Unread { viewed: false }, start);
    let (before, after) = typed(&mut model, Some(LocalInputClass::ClearPending), true, start);
    model.apply(SessionEvent::Viewed, start);
    model.apply(
        SessionEvent::InputWriteFailed {
            before: Box::new(before),
            after: Box::new(after),
        },
        start,
    );
    assert_eq!(model.activity(), IDLE);
    assert!(model.status().unread_since.is_none());
    assert!(!model.completion_armed());
    assert!(!model.submit_busy_due(later(start, SUBMIT_TURN_START_WINDOW).monotonic));

    // A submit dropped after the agent confirmed a real turn leaves that turn and its completion
    // standing, though they match what the submit had set.
    let mut model = hook_idle(start);
    let (before, after) = typed(&mut model, Some(LocalInputClass::ClearPending), true, start);
    let working = later(start, Duration::from_secs(1));
    observe(&mut model, hook(Activity::Working), working);
    assert_eq!(model.activity(), BUSY);
    model.apply(
        SessionEvent::InputWriteFailed {
            before: Box::new(before),
            after: Box::new(after),
        },
        working,
    );
    assert_eq!(model.activity(), BUSY);
    assert_eq!(model.status().observation.activity, Activity::Working);
    assert!(model.completion_armed());

    // A draft dropped while the agent reported a turn of its own leaves that turn standing.
    let mut model = hook_idle(start);
    let (before, after) = typed(&mut model, Some(LocalInputClass::SetPending), false, start);
    assert!(model.local_input_pending());
    let working = later(start, Duration::from_secs(1));
    observe(&mut model, hook(Activity::Working), working);
    model.apply(
        SessionEvent::InputWriteFailed {
            before: Box::new(before),
            after: Box::new(after),
        },
        working,
    );
    assert_eq!(model.activity(), BUSY);
    assert_eq!(model.status().observation.activity, Activity::Working);
    assert!(!model.local_input_pending());
}

#[test]
fn a_failed_second_write_keeps_the_first_deadline() {
    let start = now();
    let second = later(start, Duration::from_millis(9_900));
    let first_due = later(start, SUBMIT_TURN_START_WINDOW);
    let mut model = hook_idle(start);
    submit(&mut model, start);
    let checkpoint = model.input_checkpoint();
    submit(&mut model, second);
    assert!(!model.submit_busy_due(first_due.monotonic));
    let after = model.input_checkpoint();
    model.apply(
        SessionEvent::InputWriteFailed {
            before: Box::new(checkpoint),
            after: Box::new(after),
        },
        second,
    );
    assert_eq!(model.activity(), BUSY, "the first submit still stands");
    assert!(model.submit_busy_due(first_due.monotonic));
    assert!(expire(&mut model, first_due).publication.is_some());
    assert_eq!(model.activity(), IDLE);
}

#[test]
fn interruption_guards_neither_drop_nor_strand_background_work() {
    use agent::AgentEvent as A;
    let batch = |events| A::Batch {
        runtime: crate::model::Runtime::Codex,
        events,
    };
    for clear in [A::TurnStarted, A::StartupReady, A::SessionEnded] {
        let mut model = agent::AgentModel::default();
        model.reduce(batch(vec![A::TurnStarted]), 1);
        model.reduce(
            batch(vec![A::LocalCancel {
                kind: ESCAPE_INTERRUPT,
            }]),
            1,
        );
        assert_eq!(model.value.outcome, Some(TurnOutcome::Interrupted));
        let ended = model
            .reduce(
                batch(vec![
                    A::TurnEnded {
                        outcome: TurnOutcome::Completed,
                    },
                    A::BackgroundWork { pending: true },
                ]),
                1,
            )
            .unwrap();
        assert!(ended.background, "the rejected TurnEnded keeps its report");
        assert!(model.reduce(batch(vec![clear]), 1).is_some());
        assert!(!model.value.background);
    }
}

#[test]
fn codex_program_turn_confirmed_and_transcript_outcomes_cooperate_with_osc7501() {
    use agent::AgentEvent as A;
    use runner_terminal::program_status::{Report, State as P, Update};

    let report = |state: P, kind: Option<&str>| Report {
        id: String::new(),
        state,
        app: Some("codex".into()),
        kind: kind.map(str::to_owned),
        progress: None,
        title: None,
        message: None,
    };
    let mut seq = 0u64;
    let mut emit = |model: &mut SessionModel, state: P, kind: Option<&str>, at: Now| {
        seq += 1;
        let current = (state != P::Clear).then(|| report(state, kind));
        let trigger = Some(report(state, kind));
        model.apply(
            SessionEvent::ProgramStatus {
                seq,
                subseq: 0,
                update: Box::new(Update {
                    current: current.clone(),
                    root: current,
                    active_children: Vec::new(),
                    trigger,
                }),
                live: true,
            },
            at,
        )
    };

    let start = now();
    let mut model = SessionModel::default();
    model.set_program_status_authority(true);
    model.set_program_status_root_app(Some("codex"));
    model.set_program_status_transcript_outcomes(true);

    let confirm = |model: &mut SessionModel, conversation_changed: bool, at: Now| {
        model.apply(
            SessionEvent::Agent {
                event: A::ProgramTurnConfirmed {
                    conversation_changed,
                    prompt: None,
                },
                live: true,
            },
            at,
        )
    };

    // 1. UserPromptSubmit (ProgramTurnConfirmed) before Working counts turn_started once
    // and clears submit_busy so SubmitTimeout cannot drop a confirmed turn back to Idle.
    emit(&mut model, P::Idle, None, start);
    let rev = model.revision();
    model.apply(SessionEvent::Delivered { revision: rev }, start);
    assert!(model.submit_armed());
    let confirmed = confirm(&mut model, false, start);
    assert!(confirmed.program_turn_started);
    assert!(!model.submit_armed());
    assert!(!model.submit_busy_due(later(start, SUBMIT_TURN_START_WINDOW).monotonic));
    let working = emit(&mut model, P::Working, None, start);
    assert!(
        !working.program_turn_started,
        "Working after ProgramTurnConfirmed must not double-count turn_started"
    );

    // 2. Mid-turn ProgramTurnConfirmed (queued/steered turn 2 while still Working) counts again.
    assert!(confirm(&mut model, false, start).program_turn_started);

    // 3. Working before ProgramTurnConfirmed also counts once (not twice).
    emit(&mut model, P::Done, None, start);
    assert!(emit(&mut model, P::Working, None, start).program_turn_started);
    assert!(!confirm(&mut model, false, start).program_turn_started);

    // 3b. Delayed UserPromptSubmit arriving AFTER Working -> Done or Working -> Idle
    // must NOT double-count turn_started or leave the turn Confirmed; the next real turn
    // (in either order) and a mid-turn steer each get precisely one confirmation.
    emit(&mut model, P::Done, None, start);
    for end_state in [P::Done, P::Idle] {
        assert!(emit(&mut model, P::Working, None, start).program_turn_started);
        emit(&mut model, end_state, None, start);
        let rev = model.revision();
        model.apply(SessionEvent::Delivered { revision: rev }, start);
        assert!(
            !confirm(&mut model, false, start).program_turn_started,
            "delayed hook after Working -> {end_state:?} must not double-count turn_started"
        );
        assert!(
            model.submit_armed(),
            "delayed hook for finished turn must not clear a fresh submission's submit_busy"
        );
        // Next turn, hook first + mid-turn steer.
        assert!(confirm(&mut model, false, start).program_turn_started);
        assert!(!emit(&mut model, P::Working, None, start).program_turn_started);
        assert!(confirm(&mut model, false, start).program_turn_started);
        emit(&mut model, P::Done, None, start);

        // Next turn, native Working first + hook mid-turn + mid-turn steer.
        assert!(emit(&mut model, P::Working, None, start).program_turn_started);
        assert!(!confirm(&mut model, false, start).program_turn_started);
        assert!(confirm(&mut model, false, start).program_turn_started);
        emit(&mut model, P::Done, None, start);
    }

    // 3c. Hook first with stale Idle redraw before Working, and hook first ending directly
    // at Done without an intermediate Working frame: each turn gets precisely one confirmation.
    assert!(confirm(&mut model, false, start).program_turn_started);
    emit(&mut model, P::Idle, None, start);
    assert!(
        !emit(&mut model, P::Working, None, start).program_turn_started,
        "stale Idle between UserPromptSubmit and Working must not double-count"
    );
    assert!(confirm(&mut model, false, start).program_turn_started);
    emit(&mut model, P::Done, None, start);

    assert!(confirm(&mut model, false, start).program_turn_started);
    emit(&mut model, P::Done, None, start);
    assert!(emit(&mut model, P::Working, None, start).program_turn_started);
    assert!(!confirm(&mut model, false, start).program_turn_started);
    emit(&mut model, P::Done, None, start);

    // 4. Transcript Interrupted survives trailing OSC 7501 Idle in both orderings,
    // disarms completion, and clears on the next turn.
    for idle_first in [true, false] {
        let t = later(start, Duration::from_secs(1));
        assert!(confirm(&mut model, false, t).program_turn_started);
        assert!(!emit(&mut model, P::Working, None, t).program_turn_started);
        emit(&mut model, P::Blocked, Some("permission"), t);
        assert!(model.status().observation.needs_you());
        if idle_first {
            emit(&mut model, P::Idle, None, t);
            model.apply(
                SessionEvent::Agent {
                    event: A::ProgramTurnOutcome {
                        outcome: TurnOutcome::Interrupted,
                        policy: agent::ProgramOutcomePolicy::Immediate,
                    },
                    live: true,
                },
                t,
            );
        } else {
            model.apply(
                SessionEvent::Agent {
                    event: A::ProgramTurnOutcome {
                        outcome: TurnOutcome::Interrupted,
                        policy: agent::ProgramOutcomePolicy::Immediate,
                    },
                    live: true,
                },
                t,
            );
            emit(&mut model, P::Idle, None, t);
        }
        assert_eq!(model.status().observation.activity, Activity::Idle);
        assert_eq!(
            model.status().observation.outcome,
            Some(TurnOutcome::Interrupted)
        );
        assert!(!model.status().observation.needs_you());
        assert!(!model.completion_armed());
        assert_eq!(model.status().failed_since, None);
        assert!(
            !model
                .apply(SessionEvent::TakeCompletion, t)
                .completion_consumed
        );

        // Next turn clears the outcome.
        assert!(confirm(&mut model, false, t).program_turn_started);
        assert!(!emit(&mut model, P::Working, None, t).program_turn_started);
        assert_eq!(model.status().observation.outcome, None);
        emit(&mut model, P::Done, None, t);
    }

    // 4b. API failures have both OSC error and transcript task_complete(error): both
    // orderings commute through trailing OSC idle with zero completion badges and recover cleanly.
    for error_first in [true, false] {
        let t = later(start, Duration::from_secs(2));
        assert!(confirm(&mut model, false, t).program_turn_started);
        assert!(!emit(&mut model, P::Working, None, t).program_turn_started);
        assert!(model.completion_armed());
        if error_first {
            emit(&mut model, P::Error, None, t);
            model.apply(
                SessionEvent::Agent {
                    event: A::ProgramTurnOutcome {
                        outcome: TurnOutcome::Failed,
                        policy: agent::ProgramOutcomePolicy::Immediate,
                    },
                    live: true,
                },
                t,
            );
        } else {
            model.apply(
                SessionEvent::Agent {
                    event: A::ProgramTurnOutcome {
                        outcome: TurnOutcome::Failed,
                        policy: agent::ProgramOutcomePolicy::Immediate,
                    },
                    live: true,
                },
                t,
            );
            emit(&mut model, P::Error, None, t);
        }
        // Trailing OSC idle (e.g. keypress acknowledgment) preserves Failed with zero badges.
        emit(&mut model, P::Idle, None, t);
        assert_eq!(model.status().observation.activity, Activity::Idle);
        assert_eq!(
            model.status().observation.outcome,
            Some(TurnOutcome::Failed)
        );
        assert!(model.status().failed_since.is_some());
        assert!(!model.completion_armed());
        assert!(
            !model
                .apply(SessionEvent::TakeCompletion, t)
                .completion_consumed,
            "error_first={error_first}: failed turn must not leave completion armed"
        );

        // Recovery turn clears Failed and records completion once on Done.
        assert!(confirm(&mut model, false, t).program_turn_started);
        assert!(!emit(&mut model, P::Working, None, t).program_turn_started);
        assert_eq!(model.status().observation.outcome, None);
        assert_eq!(model.status().failed_since, None);
        emit(&mut model, P::Done, None, t);
        assert_eq!(
            model.status().observation.outcome,
            Some(TurnOutcome::Completed)
        );
        assert!(
            model
                .apply(SessionEvent::TakeCompletion, t)
                .completion_consumed
        );
    }

    // 4c. Delayed UserPromptSubmit after Working -> Error does not erase Failed before Idle.
    let t = later(start, Duration::from_secs(3));
    assert!(emit(&mut model, P::Working, None, t).program_turn_started);
    emit(&mut model, P::Error, None, t);
    assert!(!confirm(&mut model, false, t).program_turn_started);
    emit(&mut model, P::Idle, None, t);
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Failed)
    );

    // 4d. An actual conversation-key change (/clear) resets saved transcript outcomes in
    // both SessionStart-before-idle and idle-before-SessionStart orders.
    for session_start_first in [true, false] {
        let t = later(start, Duration::from_secs(4));
        assert!(confirm(&mut model, false, t).program_turn_started);
        assert!(!emit(&mut model, P::Working, None, t).program_turn_started);
        model.apply(
            SessionEvent::Agent {
                event: A::ProgramTurnOutcome {
                    outcome: TurnOutcome::Interrupted,
                    policy: agent::ProgramOutcomePolicy::Immediate,
                },
                live: true,
            },
            t,
        );
        assert_eq!(
            model.status().observation.outcome,
            Some(TurnOutcome::Interrupted)
        );
        if session_start_first {
            model.apply(
                SessionEvent::Agent {
                    event: A::ProgramSessionStarted {
                        conversation_changed: true,
                        compacted: false,
                    },
                    live: true,
                },
                t,
            );
            emit(&mut model, P::Idle, None, t);
        } else {
            emit(&mut model, P::Idle, None, t);
            model.apply(
                SessionEvent::Agent {
                    event: A::ProgramSessionStarted {
                        conversation_changed: true,
                        compacted: false,
                    },
                    live: true,
                },
                t,
            );
        }
        assert_eq!(model.status().observation.activity, Activity::Idle);
        assert_eq!(
            model.status().observation.outcome,
            None,
            "session_start_first={session_start_first}: /clear must not carry Interrupted into the new conversation"
        );
        assert!(!model.completion_armed());
    }

    // 4e. Native OSC Working arriving before a missed-SessionStart conversation switch
    // (ConversationStart + UserPromptSubmit) preserves the counted native start so total
    // increments stay 1, and a following mid-turn steer increments once.
    for hook_conversation_changed in [false, true] {
        let t = later(start, Duration::from_secs(5));
        assert!(emit(&mut model, P::Working, None, t).program_turn_started);
        assert!(model.completion_armed());
        model.apply(
            SessionEvent::Agent {
                event: A::ProgramSessionStarted {
                    conversation_changed: true,
                    compacted: false,
                },
                live: true,
            },
            t,
        );
        assert!(
            !confirm(&mut model, hook_conversation_changed, t).program_turn_started,
            "hook_conversation_changed={hook_conversation_changed}: first confirmation after Working + ConversationStart must not double-count"
        );
        assert!(
            confirm(&mut model, false, t).program_turn_started,
            "hook_conversation_changed={hook_conversation_changed}: next real mid-turn steer must confirm once"
        );
        emit(&mut model, P::Done, None, t);
        assert!(
            model
                .apply(SessionEvent::TakeCompletion, t)
                .completion_consumed,
            "conversation-change binding during Working must keep completion armed"
        );
    }

    // 5. Fallback Codex without any OSC 7501 bytes keeps program_status == None after
    // ProgramTurnOutcome so baseline Forwarder transitions keep working on the next turn.
    let mut fallback = SessionModel::default();
    fallback.set_program_status_authority(true);
    fallback.apply(
        SessionEvent::Transition {
            state: SessionActivityState::Busy,
            source: StatusSource::Forwarder,
            live: true,
        },
        start,
    );
    fallback.apply(
        SessionEvent::Agent {
            event: A::ProgramTurnOutcome {
                outcome: TurnOutcome::Interrupted,
                policy: agent::ProgramOutcomePolicy::Immediate,
            },
            live: true,
        },
        start,
    );
    assert!(fallback.status().observation.program_status.is_none());
    assert_eq!(
        fallback.status().observation.outcome,
        Some(TurnOutcome::Interrupted)
    );
    // Trailing silence Idle preserves Interrupted.
    fallback.apply(
        SessionEvent::Transition {
            state: SessionActivityState::Idle,
            source: StatusSource::Forwarder,
            live: true,
        },
        start,
    );
    assert_eq!(
        fallback.status().observation.outcome,
        Some(TurnOutcome::Interrupted)
    );
    // Next baseline Busy still takes effect and clears Interrupted.
    let next_busy = fallback.apply(
        SessionEvent::Transition {
            state: SessionActivityState::Busy,
            source: StatusSource::Forwarder,
            live: true,
        },
        start,
    );
    assert_eq!(
        next_busy.publication,
        Some((SessionActivityState::Busy, StatusSource::Forwarder))
    );
    assert_eq!(fallback.status().observation.activity, Activity::Working);
    assert_eq!(fallback.status().observation.outcome, None);

    // 6. Authoritative Codex (program_status_root_app = Some("codex")) filters out foreign
    // root/child records while honoring unscoped root clear; other runtimes
    // (program_status_root_app = None) preserve the shared hierarchical update.current policy.
    let mut codex = SessionModel::default();
    codex.set_program_status_authority(true);
    codex.set_program_status_root_app(Some("codex"));
    let mut agy = SessionModel::default();
    agy.set_program_status_authority(true);
    let foreign_child = Report {
        id: "sub".into(),
        state: P::Blocked,
        app: Some("other".into()),
        kind: Some("question".into()),
        progress: None,
        title: None,
        message: None,
    };
    let codex_done = Report {
        id: String::new(),
        state: P::Done,
        app: Some("codex".into()),
        kind: None,
        progress: None,
        title: None,
        message: None,
    };
    let agy_done = Report {
        id: String::new(),
        state: P::Done,
        app: Some("agy".into()),
        kind: None,
        progress: None,
        title: None,
        message: None,
    };
    for (target, done) in [
        (&mut codex, codex_done.clone()),
        (&mut agy, agy_done.clone()),
    ] {
        let work = Report {
            state: P::Working,
            ..done.clone()
        };
        target.apply(
            SessionEvent::ProgramStatus {
                seq: 1,
                subseq: 0,
                update: Box::new(Update {
                    current: Some(work.clone()),
                    root: Some(work.clone()),
                    active_children: Vec::new(),
                    trigger: Some(work),
                }),
                live: true,
            },
            start,
        );
        target.apply(
            SessionEvent::ProgramStatus {
                seq: 2,
                subseq: 0,
                update: Box::new(Update {
                    current: Some(foreign_child.clone()),
                    root: Some(Report {
                        state: P::Working,
                        ..done.clone()
                    }),
                    active_children: vec![runner_terminal::program_status::ChildCount {
                        app: foreign_child.app.clone(),
                        active: 1,
                    }],
                    trigger: Some(foreign_child.clone()),
                }),
                live: true,
            },
            start,
        );
        // Foreign child preempts update.current while root emits Done.
        target.apply(
            SessionEvent::ProgramStatus {
                seq: 3,
                subseq: 0,
                update: Box::new(Update {
                    current: Some(foreign_child.clone()),
                    root: Some(done.clone()),
                    active_children: vec![runner_terminal::program_status::ChildCount {
                        app: foreign_child.app.clone(),
                        active: 1,
                    }],
                    trigger: Some(done),
                }),
                live: true,
            },
            start,
        );
    }
    assert_eq!(
        codex.status().observation.outcome,
        Some(TurnOutcome::Completed),
        "authoritative Codex must ignore foreign child and accept its own root Done"
    );
    assert!(!codex.status().observation.needs_you());
    assert!(
        agy.status().observation.needs_you(),
        "non-Codex runtimes keep shared hierarchical update.current policy"
    );
    assert_eq!(agy.status().observation.outcome, None);

    // 7. Antigravity dismisses OSC 7501 error on trailing idle to plain Idle (outcome == None),
    // whereas Codex, Claude Code, and pi reconcile transcript/OSC errors and preserve Failed
    // until new work.
    for (runtime, app, expected_transcript_outcomes) in [
        (crate::model::Runtime::Antigravity, "agy", false),
        (crate::model::Runtime::Codex, "codex", true),
        (crate::model::Runtime::ClaudeCode, "claude-code", true),
        (crate::model::Runtime::Pi, "pi", true),
        (crate::model::Runtime::Grok, "grok", true),
        (crate::model::Runtime::OpenCode, "opencode", true),
    ] {
        let semantics = crate::runtimes::adapter(runtime).status_semantics();
        assert_eq!(
            semantics.program_status_transcript_outcomes, expected_transcript_outcomes,
            "{runtime:?}"
        );
        let mut other = SessionModel::default();
        other.set_program_status_authority(semantics.program_status_authoritative(false));
        other.set_program_status_root_app(semantics.program_status_root_app);
        other.set_program_status_transcript_outcomes(semantics.program_status_transcript_outcomes);
        other.set_program_status_cancel_interrupt(semantics.program_status_cancel_interrupt);
        other.set_program_status_idle_completes(semantics.program_status_idle_completes);
        other
            .set_program_status_suppress_child_tasks(semantics.program_status_suppress_child_tasks);
        for (seq, state) in [(1, P::Working), (2, P::Error), (3, P::Idle)] {
            let report = Report {
                id: String::new(),
                state,
                app: Some(app.into()),
                kind: None,
                progress: None,
                title: None,
                message: None,
            };
            other.apply(
                SessionEvent::ProgramStatus {
                    seq,
                    subseq: 0,
                    update: Box::new(Update {
                        current: Some(report.clone()),
                        root: Some(report.clone()),
                        active_children: Vec::new(),
                        trigger: Some(report),
                    }),
                    live: true,
                },
                start,
            );
        }
        assert_eq!(
            other.status().observation.activity,
            Activity::Idle,
            "{runtime:?}"
        );
        assert_eq!(
            other.status().observation.outcome,
            expected_transcript_outcomes.then_some(TurnOutcome::Failed),
            "{runtime:?}: error -> idle outcome"
        );
        assert_eq!(
            other.status().failed_since.is_some(),
            expected_transcript_outcomes,
            "{runtime:?}"
        );
    }
}

#[test]
fn codex_turn_confirmation_handles_every_interleaving_of_identity_and_osc7501() {
    use agent::AgentEvent as A;
    use runner_terminal::program_status::{Report, State as P, Update};

    #[derive(Clone, Copy, Debug)]
    enum Step {
        SessionStart { changed: bool },
        UserPrompt { changed: bool },
        TurnFinished,
        TurnInterrupted,
        TurnFailed,
        Working,
        Idle,
        Done,
        Error,
    }

    fn interleavings(a: &[Step], b: &[Step]) -> Vec<Vec<Step>> {
        if a.is_empty() {
            return vec![b.to_vec()];
        }
        if b.is_empty() {
            return vec![a.to_vec()];
        }
        let mut out = Vec::new();
        for mut rest in interleavings(&a[1..], b) {
            let mut seq = Vec::with_capacity(a.len() + b.len());
            seq.push(a[0]);
            seq.append(&mut rest);
            out.push(seq);
        }
        for mut rest in interleavings(a, &b[1..]) {
            let mut seq = Vec::with_capacity(a.len() + b.len());
            seq.push(b[0]);
            seq.append(&mut rest);
            out.push(seq);
        }
        out
    }

    let apply_step = |model: &mut SessionModel, seq: &mut u64, step: Step, at: Now| -> bool {
        match step {
            Step::SessionStart { changed } => {
                model
                    .apply(
                        SessionEvent::Agent {
                            event: A::ProgramSessionStarted {
                                conversation_changed: changed,
                                compacted: false,
                            },
                            live: true,
                        },
                        at,
                    )
                    .program_turn_started
            }
            Step::UserPrompt { changed } => {
                model
                    .apply(
                        SessionEvent::Agent {
                            event: A::ProgramTurnConfirmed {
                                conversation_changed: changed,
                                prompt: None,
                            },
                            live: true,
                        },
                        at,
                    )
                    .program_turn_started
            }
            Step::TurnFinished => {
                model
                    .apply(
                        SessionEvent::Agent {
                            event: A::ProgramTurnFinished,
                            live: true,
                        },
                        at,
                    )
                    .program_turn_started
            }
            Step::TurnInterrupted | Step::TurnFailed => {
                let outcome = match step {
                    Step::TurnInterrupted => TurnOutcome::Interrupted,
                    Step::TurnFailed => TurnOutcome::Failed,
                    _ => unreachable!(),
                };
                model
                    .apply(
                        SessionEvent::Agent {
                            event: A::ProgramTurnOutcome {
                                outcome,
                                policy: agent::ProgramOutcomePolicy::Immediate,
                            },
                            live: true,
                        },
                        at,
                    )
                    .program_turn_started
            }
            Step::Working | Step::Idle | Step::Done | Step::Error => {
                *seq += 1;
                let state = match step {
                    Step::Working => P::Working,
                    Step::Idle => P::Idle,
                    Step::Done => P::Done,
                    Step::Error => P::Error,
                    _ => unreachable!(),
                };
                let report = Report {
                    id: String::new(),
                    state,
                    app: Some("codex".into()),
                    kind: None,
                    progress: None,
                    title: None,
                    message: None,
                };
                model
                    .apply(
                        SessionEvent::ProgramStatus {
                            seq: *seq,
                            subseq: 0,
                            update: Box::new(Update {
                                current: Some(report.clone()),
                                root: Some(report.clone()),
                                active_children: Vec::new(),
                                trigger: Some(report),
                            }),
                            live: true,
                        },
                        at,
                    )
                    .program_turn_started
            }
        }
    };

    let start = now();
    let cases = [
        (
            "fresh-launch-completed",
            false,
            vec![
                Step::SessionStart { changed: false },
                Step::UserPrompt { changed: false },
                Step::TurnFinished,
            ],
            vec![Step::Working, Step::Done],
            TurnOutcome::Completed,
            true,
        ),
        (
            "session-start-rekey-completed",
            true,
            vec![
                Step::SessionStart { changed: true },
                Step::UserPrompt { changed: false },
                Step::TurnFinished,
            ],
            vec![Step::Working, Step::Done],
            TurnOutcome::Completed,
            true,
        ),
        (
            "missed-session-start-rekey-completed",
            true,
            vec![
                Step::SessionStart { changed: true },
                Step::UserPrompt { changed: true },
                Step::TurnFinished,
            ],
            vec![Step::Working, Step::Done],
            TurnOutcome::Completed,
            true,
        ),
        (
            "missed-session-start-rekey-failed",
            true,
            vec![
                Step::SessionStart { changed: true },
                Step::UserPrompt { changed: true },
                Step::TurnFailed,
            ],
            vec![Step::Working, Step::Error],
            TurnOutcome::Failed,
            false,
        ),
        (
            "fresh-launch-interrupted",
            false,
            vec![
                Step::SessionStart { changed: false },
                Step::UserPrompt { changed: false },
                Step::TurnInterrupted,
            ],
            vec![Step::Working, Step::Idle],
            TurnOutcome::Interrupted,
            false,
        ),
        (
            "rekey-interrupted",
            true,
            vec![
                Step::SessionStart { changed: true },
                Step::UserPrompt { changed: false },
                Step::TurnInterrupted,
            ],
            vec![Step::Working, Step::Idle],
            TurnOutcome::Interrupted,
            false,
        ),
    ];

    for (name, prior_interrupted, stream_a, stream_b, expected_outcome, expect_completion) in cases
    {
        let all = interleavings(&stream_a, &stream_b);
        assert_eq!(all.len(), 10, "{name}: C(5,2) = 10 interleavings");
        for trace in all {
            let mut model = SessionModel::default();
            model.set_program_status_authority(true);
            model.set_program_status_root_app(Some("codex"));
            model.set_program_status_transcript_outcomes(true);
            let mut seq = 0u64;
            if prior_interrupted {
                model.apply(
                    SessionEvent::Agent {
                        event: A::ProgramTurnOutcome {
                            outcome: TurnOutcome::Interrupted,
                            policy: agent::ProgramOutcomePolicy::Immediate,
                        },
                        live: true,
                    },
                    start,
                );
            }

            let mut starts = 0usize;
            for &step in &trace {
                if apply_step(&mut model, &mut seq, step, start) {
                    starts += 1;
                }
            }
            assert_eq!(
                starts, 1,
                "{name} trace={trace:?}: single turn must count exactly once"
            );
            assert_eq!(
                model.status().observation.outcome,
                Some(expected_outcome),
                "{name} trace={trace:?}: native outcome must be retained"
            );
            assert_eq!(
                model
                    .apply(SessionEvent::TakeCompletion, start)
                    .completion_consumed,
                expect_completion,
                "{name} trace={trace:?}: completion badge mismatch"
            );
            if expected_outcome == TurnOutcome::Failed {
                // Trailing OSC idle preserves Failed across every failed interleaving.
                seq += 1;
                let idle = Report {
                    id: String::new(),
                    state: P::Idle,
                    app: Some("codex".into()),
                    kind: None,
                    progress: None,
                    title: None,
                    message: None,
                };
                model.apply(
                    SessionEvent::ProgramStatus {
                        seq,
                        subseq: 0,
                        update: Box::new(Update {
                            current: Some(idle.clone()),
                            root: Some(idle.clone()),
                            active_children: Vec::new(),
                            trigger: Some(idle),
                        }),
                        live: true,
                    },
                    start,
                );
                assert_eq!(
                    model.status().observation.outcome,
                    Some(TurnOutcome::Failed),
                    "{name} trace={trace:?}: trailing idle must preserve Failed"
                );
            }

            // Subsequent genuine Turn 2 + mid-turn steer (tested in both Working-first and
            // hook-first starts) must clear any prior outcome and count precisely once each.
            let t2 = later(start, Duration::from_secs(1));
            assert!(
                apply_step(&mut model, &mut seq, Step::Working, t2),
                "{name} trace={trace:?}: Turn 2 Working must start once"
            );
            assert_eq!(
                model.status().observation.outcome,
                None,
                "{name} trace={trace:?}: genuine Turn 2 Working must clear prior outcome"
            );
            assert!(
                !apply_step(
                    &mut model,
                    &mut seq,
                    Step::UserPrompt { changed: false },
                    t2
                ),
                "{name} trace={trace:?}: Turn 2 UserPromptSubmit must pair with Working"
            );
            assert!(
                apply_step(
                    &mut model,
                    &mut seq,
                    Step::UserPrompt { changed: false },
                    t2
                ),
                "{name} trace={trace:?}: Turn 2 mid-turn steer must confirm once"
            );
            assert!(!apply_step(&mut model, &mut seq, Step::Done, t2));
            assert_eq!(
                model.status().observation.outcome,
                Some(TurnOutcome::Completed),
                "{name} trace={trace:?}: recovery Turn 2 must finish Completed"
            );

            // Also verify hook-first recovery after an Interrupted turn clears the outcome on Turn 3.
            if expected_outcome == TurnOutcome::Interrupted {
                let t3 = later(start, Duration::from_secs(2));
                assert!(apply_step(
                    &mut model,
                    &mut seq,
                    Step::UserPrompt { changed: false },
                    t3
                ));
                assert!(!apply_step(&mut model, &mut seq, Step::TurnInterrupted, t3));
                assert!(!apply_step(&mut model, &mut seq, Step::Working, t3));
                assert!(!apply_step(&mut model, &mut seq, Step::Idle, t3));
                assert_eq!(
                    model.status().observation.outcome,
                    Some(TurnOutcome::Interrupted)
                );
                let t4 = later(start, Duration::from_secs(3));
                assert!(apply_step(
                    &mut model,
                    &mut seq,
                    Step::UserPrompt { changed: false },
                    t4
                ));
                assert!(!apply_step(&mut model, &mut seq, Step::Working, t4));
                assert_eq!(model.status().observation.outcome, None);
                assert!(!apply_step(&mut model, &mut seq, Step::Idle, t4));
                assert_eq!(
                    model.status().observation.outcome,
                    None,
                    "{name} trace={trace:?}: hook-first Turn 4 Idle must not inherit stale Interrupted"
                );
            }
        }
    }

    // Manual /compact (Working -> Done -> SessionStart(compacted=true), no UserPromptSubmit)
    // clears its unpaired native turn credit so a later Turn 2 + steer each count once.
    let mut compacted = SessionModel::default();
    compacted.set_program_status_authority(true);
    compacted.set_program_status_root_app(Some("codex"));
    compacted.set_program_status_transcript_outcomes(true);
    let mut seq = 0u64;
    assert!(apply_step(&mut compacted, &mut seq, Step::Working, start));
    assert!(!apply_step(&mut compacted, &mut seq, Step::Done, start));
    compacted.apply(
        SessionEvent::Agent {
            event: A::ProgramSessionStarted {
                conversation_changed: false,
                compacted: true,
            },
            live: true,
        },
        start,
    );
    assert!(apply_step(
        &mut compacted,
        &mut seq,
        Step::UserPrompt { changed: false },
        start
    ));
    assert!(!apply_step(&mut compacted, &mut seq, Step::Working, start));
    assert!(apply_step(
        &mut compacted,
        &mut seq,
        Step::UserPrompt { changed: false },
        start
    ));
}

#[test]
fn claude_child_records_only_add_background_task_detail_and_never_alter_root_status() {
    use runner_terminal::program_status::{Report, State as P, Update};

    let mut model = SessionModel::default();
    let semantics = crate::runtimes::adapter(crate::model::Runtime::ClaudeCode).status_semantics();
    model.set_program_status_authority(semantics.program_status_authority);
    model.set_program_status_root_app(semantics.program_status_root_app);
    model.set_program_status_transcript_outcomes(semantics.program_status_transcript_outcomes);
    model.set_program_status_cancel_interrupt(semantics.program_status_cancel_interrupt);

    let report = |id: &str, app: Option<&str>, state: P, kind: Option<&str>| Report {
        id: id.into(),
        state,
        app: app.map(str::to_owned),
        kind: kind.map(str::to_owned),
        progress: None,
        title: None,
        message: None,
    };
    let start = now();
    let mut seq = 0u64;
    let mut apply = |model: &mut SessionModel,
                     root: Option<Report>,
                     children: Vec<Report>,
                     trigger: Option<Report>| {
        seq += 1;
        let current = children
            .iter()
            .find(|r| r.state == P::Blocked)
            .cloned()
            .or_else(|| root.clone())
            .or_else(|| children.first().cloned());
        let mut active_children: Vec<runner_terminal::program_status::ChildCount> = Vec::new();
        for child in &children {
            if matches!(child.state, P::Working | P::Blocked) {
                if let Some(entry) = active_children.iter_mut().find(|e| e.app == child.app) {
                    entry.active += 1;
                } else {
                    active_children.push(runner_terminal::program_status::ChildCount {
                        app: child.app.clone(),
                        active: 1,
                    });
                }
            }
        }
        model.apply(
            SessionEvent::ProgramStatus {
                seq,
                subseq: 0,
                update: Box::new(Update {
                    current,
                    root,
                    active_children,
                    trigger,
                }),
                live: true,
            },
            start,
        )
    };

    // Start root turn.
    let root_working = report("", Some("claude-code"), P::Working, None);
    let started = apply(
        &mut model,
        Some(root_working.clone()),
        vec![],
        Some(root_working.clone()),
    );
    assert!(started.program_turn_started);
    assert_eq!(model.status().observation.activity, Activity::Working);
    assert_eq!(
        model
            .status()
            .observation
            .program_status
            .as_ref()
            .unwrap()
            .background_tasks,
        0
    );

    // Foreign child and foreign root reports are ignored completely.
    let foreign_child = report("other-1", Some("other"), P::Blocked, Some("permission"));
    let foreign = apply(
        &mut model,
        Some(root_working.clone()),
        vec![foreign_child.clone()],
        Some(foreign_child),
    );
    assert!(foreign.publication.is_none());
    assert!(!model.status().observation.needs_you());
    assert_eq!(
        model
            .status()
            .observation
            .program_status
            .as_ref()
            .unwrap()
            .background_tasks,
        0
    );

    // Claude child working + blocked records only increment background_tasks detail text;
    // root stays Working with no WaitReason and completion still armed.
    let bg1 = report("bg-1", Some("claude-code"), P::Working, None);
    let bg2 = report("bg-2", Some("claude-code"), P::Blocked, Some("permission"));
    let rev_before = model.revision();
    let c1 = apply(
        &mut model,
        Some(root_working.clone()),
        vec![bg1.clone()],
        Some(bg1.clone()),
    );
    assert!(c1.publication.is_some());
    assert!(!c1.program_turn_started);
    assert_eq!(
        model.revision(),
        rev_before,
        "child record must not bump activity_revision"
    );
    assert_eq!(model.status().observation.activity, Activity::Working);
    assert!(!model.status().observation.needs_you());
    assert_eq!(
        model
            .status()
            .observation
            .program_status
            .as_ref()
            .unwrap()
            .background_tasks,
        1
    );

    let c2 = apply(
        &mut model,
        Some(root_working.clone()),
        vec![bg1.clone(), bg2.clone()],
        Some(bg2.clone()),
    );
    assert!(c2.publication.is_some());
    assert_eq!(model.revision(), rev_before);
    assert_eq!(model.status().observation.activity, Activity::Working);
    assert!(!model.status().observation.needs_you());
    assert!(model.completion_armed());
    assert_eq!(
        model
            .status()
            .observation
            .program_status
            .as_ref()
            .unwrap()
            .background_tasks,
        2
    );

    // Child error or done never turns root Failed/Completed or disarms completion.
    let bg1_err = report("bg-1", Some("claude-code"), P::Error, None);
    apply(
        &mut model,
        Some(root_working.clone()),
        vec![bg1_err.clone(), bg2.clone()],
        Some(bg1_err),
    );
    assert_eq!(model.status().observation.activity, Activity::Working);
    assert_eq!(model.status().observation.outcome, None);
    assert!(model.completion_armed());
    assert_eq!(
        model
            .status()
            .observation
            .program_status
            .as_ref()
            .unwrap()
            .background_tasks,
        1
    );

    // Root done still marks Completed and consumes unread completion even with a child active.
    let root_done = report("", Some("claude-code"), P::Done, None);
    let done = apply(
        &mut model,
        Some(root_done.clone()),
        vec![bg2.clone()],
        Some(root_done.clone()),
    );
    assert_eq!(
        done.publication,
        Some((SessionActivityState::Idle, StatusSource::ProgramStatus))
    );
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Completed)
    );
    assert!(!model.status().observation.needs_you());
    assert_eq!(
        model
            .status()
            .observation
            .program_status
            .as_ref()
            .unwrap()
            .background_tasks,
        1
    );
    assert!(
        model
            .apply(SessionEvent::TakeCompletion, start)
            .completion_consumed
    );

    // Unscoped state=clear:id=bg-2 drops the last background task without erasing Completed.
    let clear_bg2 = report("bg-2", None, P::Clear, None);
    let cleared = apply(&mut model, Some(root_done.clone()), vec![], Some(clear_bg2));
    assert!(cleared.publication.is_some());
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Completed)
    );
    assert_eq!(
        model
            .status()
            .observation
            .program_status
            .as_ref()
            .unwrap()
            .background_tasks,
        0
    );

    // Mixed-record shell boundary 1: surviving Done root with cleared active children
    // must reset background_tasks to 0 without erasing Completed.
    apply(
        &mut model,
        Some(root_done.clone()),
        vec![bg1.clone(), bg2.clone()],
        Some(bg2.clone()),
    );
    assert_eq!(
        model
            .status()
            .observation
            .program_status
            .as_ref()
            .unwrap()
            .background_tasks,
        2
    );
    let boundary_done = apply(&mut model, Some(root_done), vec![], None);
    assert!(boundary_done.publication.is_some());
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Completed)
    );
    assert_eq!(
        model
            .status()
            .observation
            .program_status
            .as_ref()
            .unwrap()
            .background_tasks,
        0
    );

    // Mixed-record shell boundary 2: cleared Blocked root with a surviving terminal child
    // (update.current is still Some(child)) must release the root wait.
    let root_blocked = report("", Some("claude-code"), P::Blocked, Some("permission"));
    let bg_done = report("bg-done", Some("claude-code"), P::Done, None);
    apply(
        &mut model,
        Some(root_blocked.clone()),
        vec![bg_done.clone()],
        Some(root_blocked),
    );
    assert!(model.status().observation.needs_you());
    let boundary_cleared = apply(&mut model, None, vec![bg_done], None);
    assert!(boundary_cleared.publication.is_some());
    assert!(!model.status().observation.needs_you());
    assert!(model.status().observation.program_status.is_none());
}

#[test]
fn claude_and_pi_cancel_key_converts_idle_and_pi_abort_error_into_interrupted_without_badge() {
    use runner_terminal::program_status::{Report, State as P, Update};

    let start = now();
    for (runtime, app) in [
        (crate::model::Runtime::ClaudeCode, "claude-code"),
        (crate::model::Runtime::Pi, "pi"),
    ] {
        let semantics = crate::runtimes::adapter(runtime).status_semantics();
        assert!(semantics.program_status_cancel_interrupt, "{runtime:?}");
        let mut model = SessionModel::default();
        model.set_program_status_authority(semantics.program_status_authority);
        model.set_program_status_root_app(semantics.program_status_root_app);
        model.set_program_status_transcript_outcomes(semantics.program_status_transcript_outcomes);
        model.set_program_status_cancel_interrupt(semantics.program_status_cancel_interrupt);

        let mut seq = 0u64;
        let mut emit = |model: &mut SessionModel, state: P, msg: Option<&str>| {
            seq += 1;
            let report = Report {
                id: String::new(),
                state,
                app: Some(app.into()),
                kind: None,
                progress: None,
                title: None,
                message: msg.map(str::to_owned),
            };
            model.apply(
                SessionEvent::ProgramStatus {
                    seq,
                    subseq: 0,
                    update: Box::new(Update {
                        current: Some(report.clone()),
                        root: Some(report.clone()),
                        active_children: Vec::new(),
                        trigger: Some(report),
                    }),
                    live: true,
                },
                start,
            )
        };

        // 1. Esc mid-turn + OSC 7501 idle settles Interrupted with zero completion badge,
        // even when no transcript abort line is emitted (e.g. pi mid-tool abort).
        emit(&mut model, P::Working, None);
        assert!(model.completion_armed());
        model.apply(
            SessionEvent::Input {
                class: None,
                submitted: false,
                cancel: true,
            },
            start,
        );
        emit(&mut model, P::Idle, None);
        assert_eq!(
            model.status().observation.activity,
            Activity::Idle,
            "{runtime:?}"
        );
        assert_eq!(
            model.status().observation.outcome,
            Some(TurnOutcome::Interrupted),
            "{runtime:?}"
        );
        assert!(!model.completion_armed(), "{runtime:?}");
        assert_eq!(model.status().failed_since, None, "{runtime:?}");
        assert!(
            !model
                .apply(SessionEvent::TakeCompletion, start)
                .completion_consumed,
            "{runtime:?}"
        );

        // 2. Failed write rolls back cancel_pending so a dropped Esc cannot misclassify a turn.
        emit(&mut model, P::Working, None);
        let before = model.input_checkpoint();
        model.apply(
            SessionEvent::Input {
                class: None,
                submitted: false,
                cancel: true,
            },
            start,
        );
        let after = model.input_checkpoint();
        model.apply(
            SessionEvent::InputWriteFailed {
                before: Box::new(before),
                after: Box::new(after),
            },
            start,
        );
        emit(&mut model, P::Idle, None);
        assert_eq!(
            model.status().observation.outcome,
            None,
            "{runtime:?}: rolled-back Esc must not fabricate Interrupted"
        );

        // 3. pi bug #784: state=error("This operation was aborted") after Esc settles
        // Interrupted in BOTH channel orders (OSC before transcript AND transcript before OSC)
        // and survives trailing idle, while genuine errors still settle Failed.
        for transcript_first in [false, true] {
            emit(&mut model, P::Working, None);
            model.apply(
                SessionEvent::Input {
                    class: None,
                    submitted: false,
                    cancel: true,
                },
                start,
            );
            let transcript_abort = SessionEvent::Agent {
                event: agent::AgentEvent::ProgramTurnOutcome {
                    outcome: TurnOutcome::Interrupted,
                    policy: agent::ProgramOutcomePolicy::DeferWhileNativeActive,
                },
                live: true,
            };
            if transcript_first {
                model.apply(transcript_abort, start);
                assert_eq!(
                    model.status().observation.activity,
                    Activity::Working,
                    "{runtime:?}: deferred transcript abort must keep active root until native boundary"
                );
                emit(&mut model, P::Error, Some("This operation was aborted"));
            } else {
                emit(&mut model, P::Error, Some("This operation was aborted"));
                model.apply(transcript_abort, start);
            }
            assert_eq!(
                model.status().observation.outcome,
                Some(TurnOutcome::Interrupted),
                "{runtime:?} transcript_first={transcript_first}: abort-shaped error must settle Interrupted"
            );
            assert_eq!(model.status().failed_since, None, "{runtime:?}");
            assert!(
                !model
                    .apply(SessionEvent::TakeCompletion, start)
                    .completion_consumed,
                "{runtime:?} transcript_first={transcript_first}"
            );
            emit(&mut model, P::Idle, None);
            assert_eq!(
                model.status().observation.outcome,
                Some(TurnOutcome::Interrupted),
                "{runtime:?} transcript_first={transcript_first}: trailing idle preserves Interrupted"
            );
            assert_eq!(model.status().failed_since, None, "{runtime:?}");
        }

        // 3b. A genuine error after Esc + transcript abort (or an abort-shaped error while
        // idle / on a fresh uncancelled turn) must still settle Failed.
        emit(&mut model, P::Working, None);
        model.apply(
            SessionEvent::Input {
                class: None,
                submitted: false,
                cancel: true,
            },
            start,
        );
        model.apply(
            SessionEvent::Agent {
                event: agent::AgentEvent::ProgramTurnOutcome {
                    outcome: TurnOutcome::Interrupted,
                    policy: agent::ProgramOutcomePolicy::DeferWhileNativeActive,
                },
                live: true,
            },
            start,
        );
        emit(&mut model, P::Error, Some("429 rate limited"));
        assert_eq!(
            model.status().observation.outcome,
            Some(TurnOutcome::Failed),
            "{runtime:?}: genuine non-abort error must settle Failed"
        );
        assert!(model.status().failed_since.is_some(), "{runtime:?}");
        emit(&mut model, P::Idle, None);
        assert_eq!(
            model.status().observation.outcome,
            Some(TurnOutcome::Failed),
            "{runtime:?}: trailing idle preserves Failed"
        );

        // Uncancelled fresh turn hitting "This operation was aborted" settles Failed.
        emit(&mut model, P::Working, None);
        emit(&mut model, P::Error, Some("This operation was aborted"));
        assert_eq!(
            model.status().observation.outcome,
            Some(TurnOutcome::Failed),
            "{runtime:?}: uncancelled abort error without transcript abort must settle Failed"
        );
        assert!(model.status().failed_since.is_some(), "{runtime:?}");

        // 4. Next turn clears prior outcome and records Completed on done.
        emit(&mut model, P::Working, None);
        assert_eq!(model.status().observation.outcome, None, "{runtime:?}");
        assert_eq!(model.status().failed_since, None, "{runtime:?}");
        emit(&mut model, P::Done, None);
        assert_eq!(
            model.status().observation.outcome,
            Some(TurnOutcome::Completed),
            "{runtime:?}"
        );
        assert!(
            model
                .apply(SessionEvent::TakeCompletion, start)
                .completion_consumed
        );
    }
}

#[test]
fn pi_retry_error_preserves_working_root_and_claude_auth_wait_stays_authoritative() {
    use runner_terminal::program_status::{Report, State as P, Update};

    let start = now();
    let emit =
        |model: &mut SessionModel, seq: &mut u64, app: &str, state: P, kind: Option<&str>| {
            *seq += 1;
            let report = Report {
                id: String::new(),
                state,
                app: Some(app.into()),
                kind: kind.map(str::to_owned),
                progress: None,
                title: None,
                message: None,
            };
            model.apply(
                SessionEvent::ProgramStatus {
                    seq: *seq,
                    subseq: 0,
                    update: Box::new(Update {
                        current: Some(report.clone()),
                        root: Some(report.clone()),
                        active_children: Vec::new(),
                        trigger: Some(report),
                    }),
                    live: true,
                },
                start,
            )
        };

    // 1. pi transient 429 retry persists stopReason="error" while native OSC 7501 stays
    // Working and transitions straight to Done on retry success.
    let mut pi = SessionModel::default();
    let pi_sem = crate::runtimes::adapter(crate::model::Runtime::Pi).status_semantics();
    pi.set_program_status_authority(pi_sem.program_status_authority);
    pi.set_program_status_root_app(pi_sem.program_status_root_app);
    pi.set_program_status_transcript_outcomes(pi_sem.program_status_transcript_outcomes);
    pi.set_program_status_cancel_interrupt(pi_sem.program_status_cancel_interrupt);
    let mut seq = 0;
    emit(&mut pi, &mut seq, "pi", P::Working, None);
    assert!(pi.completion_armed());
    pi.apply(
        SessionEvent::Agent {
            event: agent::AgentEvent::ProgramTurnOutcome {
                outcome: TurnOutcome::Failed,
                policy: agent::ProgramOutcomePolicy::NativeFallbackOnly,
            },
            live: true,
        },
        start,
    );
    assert_eq!(pi.status().observation.activity, Activity::Working);
    assert_eq!(pi.status().observation.outcome, None);
    assert!(pi.completion_armed());
    emit(&mut pi, &mut seq, "pi", P::Done, None);
    assert_eq!(
        pi.status().observation.outcome,
        Some(TurnOutcome::Completed)
    );
    assert!(
        pi.apply(SessionEvent::TakeCompletion, start)
            .completion_consumed
    );
    emit(&mut pi, &mut seq, "pi", P::Idle, None);
    // Even a late transcript poll after Done -> Idle must not fabricate Failed.
    pi.apply(
        SessionEvent::Agent {
            event: agent::AgentEvent::ProgramTurnOutcome {
                outcome: TurnOutcome::Failed,
                policy: agent::ProgramOutcomePolicy::NativeFallbackOnly,
            },
            live: true,
        },
        start,
    );
    assert_eq!(pi.status().observation.outcome, None);
    assert_eq!(pi.status().failed_since, None);

    // 2. Claude Code native blocked:auth stays authoritative when isApiErrorMessage arrives.
    let mut claude = SessionModel::default();
    let cc_sem = crate::runtimes::adapter(crate::model::Runtime::ClaudeCode).status_semantics();
    claude.set_program_status_authority(cc_sem.program_status_authority);
    claude.set_program_status_root_app(cc_sem.program_status_root_app);
    claude.set_program_status_transcript_outcomes(cc_sem.program_status_transcript_outcomes);
    claude.set_program_status_cancel_interrupt(cc_sem.program_status_cancel_interrupt);
    let mut seq = 0;
    emit(
        &mut claude,
        &mut seq,
        "claude-code",
        P::Blocked,
        Some("auth"),
    );
    assert!(claude.status().observation.needs_you());
    claude.apply(
        SessionEvent::Agent {
            event: agent::AgentEvent::ProgramTurnOutcome {
                outcome: TurnOutcome::Failed,
                policy: agent::ProgramOutcomePolicy::NativeFallbackOnly,
            },
            live: true,
        },
        start,
    );
    assert!(claude.status().observation.needs_you());
    assert_eq!(claude.status().observation.outcome, None);

    // 3. With OSC 7501 disabled (program_status == None), NativeFallbackOnly settles Failed.
    let mut no_osc = SessionModel::default();
    no_osc.set_program_status_authority(true);
    no_osc.apply(
        SessionEvent::Transition {
            state: SessionActivityState::Busy,
            source: StatusSource::Forwarder,
            live: true,
        },
        start,
    );
    no_osc.apply(
        SessionEvent::Agent {
            event: agent::AgentEvent::ProgramTurnOutcome {
                outcome: TurnOutcome::Failed,
                policy: agent::ProgramOutcomePolicy::NativeFallbackOnly,
            },
            live: true,
        },
        start,
    );
    assert_eq!(no_osc.status().observation.activity, Activity::Idle);
    assert_eq!(
        no_osc.status().observation.outcome,
        Some(TurnOutcome::Failed)
    );
    assert!(no_osc.status().failed_since.is_some());
    assert!(no_osc.status().observation.program_status.is_none());
}

#[test]
fn pi_custom_prompt_wait_overlay_holds_deduplicates_clears_on_shutdown_and_keeps_osc_authority() {
    use runner_terminal::program_status::{Report, State as P, Update};

    let start = now();
    let mut model = SessionModel::default();
    let sem = crate::runtimes::adapter(crate::model::Runtime::Pi).status_semantics();
    model.set_program_status_authority(sem.program_status_authority);
    model.set_program_status_root_app(sem.program_status_root_app);
    model.set_program_status_transcript_outcomes(sem.program_status_transcript_outcomes);
    model.set_program_status_cancel_interrupt(sem.program_status_cancel_interrupt);

    let seq = std::cell::Cell::new(0u64);
    let emit = |model: &mut SessionModel, state: P, kind: Option<&str>| {
        let next = seq.get() + 1;
        seq.set(next);
        let report = Report {
            id: String::new(),
            state,
            app: Some("pi".into()),
            kind: kind.map(str::to_owned),
            progress: None,
            title: None,
            message: None,
        };
        model.apply(
            SessionEvent::ProgramStatus {
                seq: next,
                subseq: 0,
                update: Box::new(Update {
                    current: Some(report.clone()),
                    root: Some(report.clone()),
                    active_children: Vec::new(),
                    trigger: Some(report),
                }),
                live: true,
            },
            start,
        )
    };
    let overlay = |model: &mut SessionModel, active: bool| {
        model.apply(
            SessionEvent::Agent {
                event: agent::AgentEvent::ProgramWaitOverlay { active },
                live: true,
            },
            start,
        )
    };

    // 1. Unannotated custom prompt shows Needs you (Unknown) without arming hooks,
    // preserves OSC Working/Done/Failed/Interrupted authority, and releases once on end.
    emit(&mut model, P::Idle, None);
    let opened = overlay(&mut model, true);
    assert_eq!(
        opened.publication,
        Some((SessionActivityState::Busy, StatusSource::ProgramStatus))
    );
    assert!(!opened.input_cleared);
    assert_eq!(model.status().observation.interactions.len(), 1);
    assert_eq!(
        model.status().observation.interactions[0].reason,
        WaitReason::Unknown
    );
    assert_eq!(
        model.status().observation.source,
        ObservationSource::Baseline
    );
    assert!(!model.completion_armed());
    // Idempotent duplicate start does not duplicate wait.
    assert!(overlay(&mut model, true).publication.is_none());
    assert_eq!(model.status().observation.interactions.len(), 1);

    // OSC Working and Done retain authority while overlay is open and award completion once released.
    emit(&mut model, P::Working, None);
    assert_eq!(model.status().observation.activity, Activity::Working);
    assert!(model.completion_armed());
    assert_eq!(model.status().observation.interactions.len(), 1);
    emit(&mut model, P::Done, None);
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Completed)
    );
    assert_eq!(model.status().observation.interactions.len(), 1);
    let closed = overlay(&mut model, false);
    assert!(closed.input_cleared);
    assert_eq!(
        closed.publication,
        Some((SessionActivityState::Idle, StatusSource::ProgramStatus))
    );
    assert!(model.status().observation.interactions.is_empty());
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Completed)
    );
    assert!(
        model
            .apply(SessionEvent::TakeCompletion, start)
            .completion_consumed
    );

    // 2. OSC Failed and Interrupted retain authority across an open overlay.
    emit(&mut model, P::Working, None);
    overlay(&mut model, true);
    emit(&mut model, P::Error, None);
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Failed)
    );
    assert!(model.status().failed_since.is_some());
    assert_eq!(model.status().observation.interactions.len(), 1);
    assert!(overlay(&mut model, false).input_cleared);
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Failed)
    );
    assert!(model.status().failed_since.is_some());

    emit(&mut model, P::Working, None);
    overlay(&mut model, true);
    model.apply(
        SessionEvent::Input {
            class: None,
            submitted: false,
            cancel: true,
        },
        start,
    );
    emit(&mut model, P::Idle, None);
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Interrupted)
    );
    assert_eq!(model.status().observation.interactions.len(), 1);
    // session_shutdown / ProgramIdentityUnavailable also clears the overlay.
    let shut = model.apply(
        SessionEvent::Agent {
            event: agent::AgentEvent::ProgramIdentityUnavailable,
            live: true,
        },
        start,
    );
    assert!(shut.input_cleared);
    assert!(model.status().observation.interactions.is_empty());
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Interrupted)
    );

    // 3. Annotated custom prompt (both OSC blocked + ui_prompt_start) deduplicates to a single
    // wait and clears exactly once across all open/close orderings.
    for osc_open_first in [true, false] {
        for osc_close_first in [true, false] {
            emit(&mut model, P::Working, None);
            if osc_open_first {
                emit(&mut model, P::Blocked, Some("question"));
                assert_eq!(model.status().observation.interactions.len(), 1);
                assert!(overlay(&mut model, true).publication.is_none());
            } else {
                assert!(overlay(&mut model, true).publication.is_some());
                emit(&mut model, P::Blocked, Some("question"));
            }
            assert_eq!(
                model.status().observation.interactions.len(),
                1,
                "osc_open_first={osc_open_first}: annotated custom must show one wait"
            );
            assert_eq!(
                model.status().observation.interactions[0].reason,
                WaitReason::Answer
            );

            let mut clears = 0usize;
            if osc_close_first {
                if emit(&mut model, P::Idle, None).input_cleared {
                    clears += 1;
                }
                assert!(model.status().observation.needs_you());
                assert_eq!(model.status().observation.interactions.len(), 1);
                if overlay(&mut model, false).input_cleared {
                    clears += 1;
                }
            } else {
                if overlay(&mut model, false).input_cleared {
                    clears += 1;
                }
                assert!(model.status().observation.needs_you());
                assert_eq!(model.status().observation.interactions.len(), 1);
                if emit(&mut model, P::Idle, None).input_cleared {
                    clears += 1;
                }
            }
            assert_eq!(
                clears, 1,
                "osc_open_first={osc_open_first} osc_close_first={osc_close_first}: must clear once"
            );
            assert!(!model.status().observation.needs_you());
        }
    }

    // 4. Stacked dialog (confirm -> unannotated custom): confirm opens with OSC blocked +
    // outer ui_prompt_start; unannotated custom drops OSC back to Working while both stay open.
    // The outer overlay must retain Needs you (Unknown) until both close, clearing once.
    emit(&mut model, P::Working, None);
    let mut stack_clears = 0usize;
    if emit(&mut model, P::Blocked, Some("permission")).input_cleared {
        stack_clears += 1;
    }
    if overlay(&mut model, true).input_cleared {
        stack_clears += 1;
    }
    assert_eq!(model.status().observation.interactions.len(), 1);
    assert_eq!(
        model.status().observation.interactions[0].reason,
        WaitReason::Approval
    );
    // Unannotated custom opens on top -> native reporter publishes Working (no blocked).
    if emit(&mut model, P::Working, None).input_cleared {
        stack_clears += 1;
    }
    assert_eq!(
        stack_clears, 0,
        "opening stacked custom must not clear wait"
    );
    assert_eq!(model.status().observation.interactions.len(), 1);
    assert_eq!(
        model.status().observation.interactions[0].reason,
        WaitReason::Unknown
    );
    // Unannotated custom closes -> confirm returns as native Blocked(permission).
    if emit(&mut model, P::Blocked, Some("permission")).input_cleared {
        stack_clears += 1;
    }
    assert_eq!(stack_clears, 0);
    assert_eq!(model.status().observation.interactions.len(), 1);
    assert_eq!(
        model.status().observation.interactions[0].reason,
        WaitReason::Approval
    );
    // Confirm closes -> native Working then outer ui_prompt_end.
    if emit(&mut model, P::Working, None).input_cleared {
        stack_clears += 1;
    }
    assert!(model.status().observation.needs_you());
    if overlay(&mut model, false).input_cleared {
        stack_clears += 1;
    }
    assert_eq!(stack_clears, 1, "stacked dialogs must clear exactly once");
    assert!(!model.status().observation.needs_you());

    // 5. OSC state=clear (e.g. pi terminal.stop on suspend) clears native evidence only and
    // preserves an open hook wait overlay in both native-present and hook-only sessions.
    let clear_native = |model: &mut SessionModel, seq: &std::cell::Cell<u64>| {
        let next = seq.get() + 1;
        seq.set(next);
        let report = Report {
            id: String::new(),
            state: P::Clear,
            app: None,
            kind: None,
            progress: None,
            title: None,
            message: None,
        };
        model.apply(
            SessionEvent::ProgramStatus {
                seq: next,
                subseq: 0,
                update: Box::new(Update {
                    current: None,
                    root: None,
                    active_children: Vec::new(),
                    trigger: Some(report),
                }),
                live: true,
            },
            start,
        )
    };
    emit(&mut model, P::Working, None);
    overlay(&mut model, true);
    assert!(model.status().observation.program_status.is_some());
    let cleared = clear_native(&mut model, &seq);
    assert!(!cleared.input_cleared);
    assert!(model.status().observation.program_status.is_none());
    assert!(
        model.status().observation.needs_you(),
        "OSC clear must preserve active hook wait across suspend/resume"
    );
    emit(&mut model, P::Idle, None);
    assert!(model.status().observation.needs_you());
    assert!(overlay(&mut model, false).input_cleared);
    assert!(!model.status().observation.needs_you());

    // Hook-only session (no prior native observation): OSC clear must not orphan the wait.
    let mut hook_only = SessionModel::default();
    hook_only.set_program_status_authority(sem.program_status_authority);
    hook_only.set_program_status_root_app(sem.program_status_root_app);
    let hseq = std::cell::Cell::new(0u64);
    assert!(overlay(&mut hook_only, true).publication.is_some());
    assert!(hook_only.status().observation.program_status.is_none());
    assert!(hook_only.status().observation.needs_you());
    clear_native(&mut hook_only, &hseq);
    assert!(hook_only.status().observation.needs_you());
    assert!(overlay(&mut hook_only, false).input_cleared);
    assert!(!hook_only.status().observation.needs_you());

    // 6. program_done fires ONLY on a fresh authoritative root Done (even while overlay is
    // open) and NEVER on overlay open/close republications of an old Completed status.
    emit(&mut model, P::Working, None);
    overlay(&mut model, true);
    let done_eff = emit(&mut model, P::Done, None);
    assert!(
        done_eff.program_done,
        "fresh root Done during custom wait must still signal program_done"
    );
    let close_eff = overlay(&mut model, false);
    assert!(
        !close_eff.program_done,
        "closing overlay over already-settled Completed must not signal program_done"
    );
}

#[test]
fn grok_osc7501_turns_waits_cancel_subagents_and_transcript_fallback() {
    use agent::{AgentEvent as A, ProgramOutcomePolicy};
    use runner_terminal::program_status::{ChildCount, Report, State as P, Update};

    let start = now();
    let sem = crate::runtimes::adapter(crate::model::Runtime::Grok).status_semantics();
    assert!(sem.program_status_authority);
    assert_eq!(sem.program_status_root_app, Some("grok"));
    assert!(sem.program_status_transcript_outcomes);
    assert!(sem.program_status_cancel_interrupt);
    assert!(!sem.program_status_idle_completes);
    assert!(!sem.program_status_suppress_child_tasks);

    let mut model = SessionModel::default();
    model.set_program_status_authority(sem.program_status_authority);
    model.set_program_status_root_app(sem.program_status_root_app);
    model.set_program_status_transcript_outcomes(sem.program_status_transcript_outcomes);
    model.set_program_status_cancel_interrupt(sem.program_status_cancel_interrupt);
    model.set_program_status_idle_completes(sem.program_status_idle_completes);
    model.set_program_status_suppress_child_tasks(sem.program_status_suppress_child_tasks);

    let mut seq = 0u64;
    let mut emit_root = |model: &mut SessionModel,
                         state: P,
                         kind: Option<&str>,
                         msg: Option<&str>,
                         children: Vec<ChildCount>| {
        seq += 1;
        let report = Report {
            id: String::new(),
            state,
            app: Some("grok".into()),
            kind: kind.map(str::to_owned),
            progress: None,
            title: None,
            message: msg.map(str::to_owned),
        };
        let is_clear = state == P::Clear;
        model.apply(
            SessionEvent::ProgramStatus {
                seq,
                subseq: 0,
                update: Box::new(Update {
                    current: (!is_clear).then(|| report.clone()),
                    root: (!is_clear).then(|| report.clone()),
                    active_children: children,
                    trigger: Some(report),
                }),
                live: true,
            },
            start,
        )
    };

    // 1. Probed sequence: idle at start -> working ("Thinking") -> blocked:permission ->
    // working -> done ("Turn complete in 2.5s.") records Completed + program_done once.
    emit_root(&mut model, P::Idle, None, None, vec![]);
    assert_eq!(model.status().observation.activity, Activity::Idle);
    assert_eq!(model.status().observation.outcome, None);
    let started = emit_root(&mut model, P::Working, None, Some("Thinking"), vec![]);
    assert!(started.program_turn_started);
    assert_eq!(model.status().observation.activity, Activity::Working);
    assert_eq!(
        model
            .status()
            .observation
            .program_status
            .as_ref()
            .unwrap()
            .message
            .as_deref(),
        Some("Thinking")
    );

    for (kind, expected) in [
        (Some("permission"), WaitReason::Approval),
        (Some("question"), WaitReason::Answer),
        (Some("auth"), WaitReason::Unknown),
        (None, WaitReason::Unknown),
    ] {
        emit_root(&mut model, P::Blocked, kind, Some("bash: ls"), vec![]);
        assert!(model.status().observation.needs_you(), "kind={kind:?}");
        assert_eq!(
            model.status().observation.interactions[0].reason,
            expected,
            "kind={kind:?}"
        );
    }

    // Subagent child adds background_tasks detail only and never alters root status or done.
    let sub = vec![ChildCount {
        app: Some("grok".into()),
        active: 2,
    }];
    emit_root(
        &mut model,
        P::Working,
        None,
        Some("Responding"),
        sub.clone(),
    );
    assert!(!model.status().observation.needs_you());
    assert_eq!(
        model
            .status()
            .observation
            .program_status
            .as_ref()
            .unwrap()
            .background_tasks,
        2
    );
    let done = emit_root(
        &mut model,
        P::Done,
        None,
        Some("Turn complete in 2.5s."),
        sub,
    );
    assert!(done.program_done);
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Completed)
    );
    assert!(
        model
            .apply(SessionEvent::TakeCompletion, start)
            .completion_consumed
    );

    // 2. Probed Ctrl+C sequence: working -> Ctrl+C -> working("Cancelling") -> idle settles
    // Interrupted with no unread completion badge.
    emit_root(
        &mut model,
        P::Working,
        None,
        Some("Waiting for response…"),
        vec![],
    );
    model.apply(
        SessionEvent::Input {
            class: None,
            submitted: false,
            cancel: true,
        },
        start,
    );
    emit_root(&mut model, P::Working, None, Some("Cancelling"), vec![]);
    assert_eq!(model.status().observation.activity, Activity::Working);
    let cancelled = emit_root(&mut model, P::Idle, None, None, vec![]);
    assert!(!cancelled.program_done);
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Interrupted)
    );
    assert_eq!(model.status().failed_since, None);
    assert!(
        !model
            .apply(SessionEvent::TakeCompletion, start)
            .completion_consumed
    );

    // 3. Error settles Failed and survives trailing idle; clear on exit drops program_status.
    emit_root(&mut model, P::Working, None, None, vec![]);
    emit_root(&mut model, P::Error, None, Some("API error"), vec![]);
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Failed)
    );
    assert!(model.status().failed_since.is_some());
    emit_root(&mut model, P::Idle, None, None, vec![]);
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Failed)
    );
    // 3b. Grok working -> idle (without OSC error) reconciles with updates.jsonl Failed both
    // before and after idle, while live Working/Blocked and explicit OSC done stay authoritative.
    emit_root(&mut model, P::Working, None, None, vec![]);
    model.apply(
        SessionEvent::Agent {
            event: A::ProgramTurnOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
            },
            live: true,
        },
        start,
    );
    assert_eq!(model.status().observation.activity, Activity::Working);
    emit_root(&mut model, P::Blocked, Some("permission"), None, vec![]);
    assert!(model.status().observation.needs_you());
    emit_root(&mut model, P::Idle, None, None, vec![]);
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Failed)
    );
    assert!(model.status().failed_since.is_some());

    // Late updates.jsonl Failed arriving after working -> idle (no OSC error) sets Failed.
    emit_root(&mut model, P::Working, None, None, vec![]);
    emit_root(&mut model, P::Idle, None, None, vec![]);
    assert_eq!(model.status().observation.outcome, None);
    model.apply(
        SessionEvent::Agent {
            event: A::ProgramTurnOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
            },
            live: true,
        },
        start,
    );
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Failed)
    );
    assert!(model.status().failed_since.is_some());

    // Explicit OSC done stays Completed even if a stale updates.jsonl Failed arrives afterward.
    emit_root(&mut model, P::Working, None, None, vec![]);
    let explicit_done = emit_root(&mut model, P::Done, None, None, vec![]);
    assert!(explicit_done.program_done);
    model.apply(
        SessionEvent::Agent {
            event: A::ProgramTurnOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
            },
            live: true,
        },
        start,
    );
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Completed)
    );
    emit_root(&mut model, P::Clear, None, None, vec![]);
    assert!(model.status().observation.program_status.is_none());

    // 4. Grok program_status="off" (no OSC 7501) still surfaces updates.jsonl Failed outcome.
    let mut no_osc = SessionModel::default();
    no_osc.set_program_status_authority(sem.program_status_authority);
    no_osc.set_program_status_root_app(sem.program_status_root_app);
    no_osc.set_program_status_transcript_outcomes(sem.program_status_transcript_outcomes);
    no_osc.set_program_status_cancel_interrupt(sem.program_status_cancel_interrupt);
    no_osc.apply(
        SessionEvent::Transition {
            state: SessionActivityState::Busy,
            source: StatusSource::Forwarder,
            live: true,
        },
        start,
    );
    no_osc.apply(
        SessionEvent::Agent {
            event: A::ProgramTurnOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
            },
            live: true,
        },
        start,
    );
    assert_eq!(no_osc.status().observation.activity, Activity::Idle);
    assert_eq!(
        no_osc.status().observation.outcome,
        Some(TurnOutcome::Failed)
    );
    assert!(no_osc.status().failed_since.is_some());
}

#[test]
fn opencode_osc7501_idle_completion_late_outcome_reversal_multi_session_and_suppressed_children() {
    use agent::{AgentEvent as A, ProgramOutcomePolicy};
    use runner_terminal::program_status::{ChildCount, Report, State as P, Update};

    let start = now();
    let sem = crate::runtimes::adapter(crate::model::Runtime::OpenCode).status_semantics();
    assert!(sem.program_status_authority);
    assert_eq!(sem.program_status_root_app, Some("opencode"));
    assert!(sem.program_status_transcript_outcomes);
    assert!(sem.program_status_cancel_interrupt);
    assert!(sem.program_status_idle_completes);
    assert!(sem.program_status_suppress_child_tasks);

    let make_model = || {
        let mut model = SessionModel::default();
        model.set_program_status_authority(sem.program_status_authority);
        model.set_program_status_root_app(sem.program_status_root_app);
        model.set_program_status_transcript_outcomes(sem.program_status_transcript_outcomes);
        model.set_program_status_cancel_interrupt(sem.program_status_cancel_interrupt);
        model.set_program_status_idle_completes(sem.program_status_idle_completes);
        model.set_program_status_suppress_child_tasks(sem.program_status_suppress_child_tasks);
        model
    };

    let emit = |model: &mut SessionModel,
                seq: &mut u64,
                state: P,
                kind: Option<&str>,
                msg: Option<&str>,
                children: usize| {
        *seq += 1;
        let report = Report {
            id: String::new(),
            state,
            app: Some("opencode".into()),
            kind: kind.map(str::to_owned),
            progress: None,
            title: None,
            message: msg.map(str::to_owned),
        };
        let active_children = if children > 0 {
            vec![ChildCount {
                app: Some("opencode".into()),
                active: children,
            }]
        } else {
            Vec::new()
        };
        model.apply(
            SessionEvent::ProgramStatus {
                seq: *seq,
                subseq: 0,
                update: Box::new(Update {
                    current: Some(report.clone()),
                    root: Some(report.clone()),
                    active_children,
                    trigger: Some(report),
                }),
                live: true,
            },
            start,
        )
    };

    // 1. Startup Idle and startup Blocked -> Idle (never Working) must NOT fabricate Completed.
    let mut model = make_model();
    let mut seq = 0u64;
    let boot = emit(&mut model, &mut seq, P::Idle, None, None, 0);
    assert!(!boot.program_done);
    assert_eq!(model.status().observation.outcome, None);
    emit(
        &mut model,
        &mut seq,
        P::Blocked,
        Some("permission"),
        None,
        1,
    );
    assert!(model.status().observation.needs_you());
    assert_eq!(
        model.status().observation.interactions[0].reason,
        WaitReason::Approval
    );
    let unblocked = emit(&mut model, &mut seq, P::Idle, None, None, 0);
    assert!(!unblocked.program_done);
    assert_eq!(model.status().observation.outcome, None);
    assert!(
        !model
            .apply(SessionEvent::TakeCompletion, start)
            .completion_consumed
    );

    // 2. Normal single-session turn: per-session child is suppressed (background_tasks == 0),
    // root working -> idle produces Completed + program_done once; duplicate idle does not repeat.
    let started = emit(&mut model, &mut seq, P::Working, None, None, 1);
    assert!(started.program_turn_started);
    assert_eq!(
        model
            .status()
            .observation
            .program_status
            .as_ref()
            .unwrap()
            .background_tasks,
        0,
        "OpenCode single-session child must not display as a background task"
    );
    // Child-only updates while root is unchanged never alter root or background_tasks.
    seq += 1;
    let child_only = Report {
        id: "ses_abc123".into(),
        state: P::Blocked,
        app: Some("opencode".into()),
        kind: Some("permission".into()),
        progress: None,
        title: None,
        message: None,
    };
    let root_working = Report {
        id: String::new(),
        state: P::Working,
        app: Some("opencode".into()),
        kind: None,
        progress: None,
        title: None,
        message: None,
    };
    let child_eff = model.apply(
        SessionEvent::ProgramStatus {
            seq,
            subseq: 0,
            update: Box::new(Update {
                current: Some(child_only.clone()),
                root: Some(root_working),
                active_children: vec![ChildCount {
                    app: Some("opencode".into()),
                    active: 1,
                }],
                trigger: Some(child_only),
            }),
            live: true,
        },
        start,
    );
    assert!(child_eff.publication.is_none());
    assert!(!model.status().observation.needs_you());

    let done = emit(&mut model, &mut seq, P::Idle, None, None, 0);
    assert!(done.program_done);
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Completed)
    );
    assert!(
        model
            .apply(SessionEvent::TakeCompletion, start)
            .completion_consumed
    );
    model.apply(SessionEvent::Unread { viewed: false }, start);
    // Duplicate idle does not re-trigger program_done.
    let dup = emit(&mut model, &mut seq, P::Idle, None, None, 0);
    assert!(!dup.program_done);
    model.apply(SessionEvent::Viewed, start);

    // 3. Multi-session root summaries + blocked kinds (permission, question, mixed/absent).
    emit(
        &mut model,
        &mut seq,
        P::Working,
        None,
        Some("2 sessions working"),
        2,
    );
    assert_eq!(
        model
            .status()
            .observation
            .program_status
            .as_ref()
            .unwrap()
            .message
            .as_deref(),
        Some("2 sessions working")
    );
    assert_eq!(
        model
            .status()
            .observation
            .program_status
            .as_ref()
            .unwrap()
            .background_tasks,
        0
    );
    for (kind, msg, expected) in [
        (Some("permission"), Some("Run bash"), WaitReason::Approval),
        (Some("question"), Some("Which option?"), WaitReason::Answer),
        (None, Some("2 sessions need input"), WaitReason::Unknown),
    ] {
        emit(&mut model, &mut seq, P::Blocked, kind, msg, 2);
        assert!(model.status().observation.needs_you());
        assert_eq!(model.status().observation.interactions[0].reason, expected);
    }
    let multi_done = emit(&mut model, &mut seq, P::Idle, None, None, 0);
    assert!(multi_done.program_done);
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Completed)
    );
    model.apply(SessionEvent::TakeCompletion, start);

    // 4. Esc/Ctrl+C mid-turn prevents idle-inferred Completed and settles Interrupted.
    emit(&mut model, &mut seq, P::Working, None, None, 1);
    model.apply(
        SessionEvent::Input {
            class: None,
            submitted: false,
            cancel: true,
        },
        start,
    );
    let interrupted = emit(&mut model, &mut seq, P::Idle, None, None, 0);
    assert!(!interrupted.program_done);
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Interrupted)
    );
    assert!(
        !model
            .apply(SessionEvent::TakeCompletion, start)
            .completion_consumed
    );

    // 5. opencode.db Failed arriving BEFORE OSC idle is deferred until idle and prevents Completed.
    emit(&mut model, &mut seq, P::Working, None, None, 1);
    seq += 1;
    let working_child = Report {
        id: "ses_abc123".into(),
        state: P::Working,
        app: Some("opencode".into()),
        kind: None,
        progress: None,
        title: None,
        message: None,
    };
    let root_work = Report {
        id: String::new(),
        state: P::Working,
        app: Some("opencode".into()),
        kind: None,
        progress: None,
        title: None,
        message: None,
    };
    model.apply(
        SessionEvent::ProgramStatus {
            seq,
            subseq: 0,
            update: Box::new(Update {
                current: Some(working_child.clone()),
                root: Some(root_work),
                active_children: vec![ChildCount {
                    app: Some("opencode".into()),
                    active: 1,
                }],
                trigger: Some(working_child),
            }),
            live: true,
        },
        start,
    );
    model.apply(
        SessionEvent::Agent {
            event: A::ProgramSessionOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                session_id: "ses_abc123".into(),
                ended_at_ms: start.wall_millis + 10,
                superseded: false,
            },
            live: true,
        },
        start,
    );
    assert_eq!(model.status().observation.activity, Activity::Working);
    let failed_idle = emit(&mut model, &mut seq, P::Idle, None, None, 0);
    assert!(!failed_idle.program_done);
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Failed)
    );
    assert!(model.status().failed_since.is_some());
    assert!(
        !model
            .apply(SessionEvent::TakeCompletion, start)
            .completion_consumed
    );

    // 6. opencode.db Failed arriving AFTER OSC working -> idle overrides Completed and sets
    // revert_unread = true when no prior unread existed.
    emit(&mut model, &mut seq, P::Working, None, None, 1);
    let optimistic = emit(&mut model, &mut seq, P::Idle, None, None, 0);
    assert!(optimistic.program_done);
    assert!(
        model
            .apply(SessionEvent::TakeCompletion, start)
            .completion_consumed
    );
    model.apply(SessionEvent::Unread { viewed: false }, start);
    assert!(model.status().unread_since.is_some());
    let late_fail = model.apply(
        SessionEvent::Agent {
            event: A::ProgramSessionOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                session_id: "ses_abc123".into(),
                ended_at_ms: start.wall_millis + 20,
                superseded: false,
            },
            live: true,
        },
        start,
    );
    assert!(late_fail.revert_unread);
    assert_eq!(model.status().unread_since, None);
    assert_eq!(
        model.status().observation.outcome,
        Some(TurnOutcome::Failed)
    );
    assert!(model.status().failed_since.is_some());

    // 7. When Turn 1 already left an unread badge, Turn 2 records its own optimistic Unread
    // at a later timestamp; a late Failed on Turn 2 restores Turn 1's unread_since in memory
    // AND signals revert_unread = true so the manager can CAS-restore durable DB state.
    let mut prior = make_model();
    let mut pseq = 0u64;
    let t1 = start;
    let t2 = Now {
        monotonic: start.monotonic + Duration::from_secs(5),
        wall_millis: start.wall_millis + 5_000,
    };
    emit(&mut prior, &mut pseq, P::Working, None, None, 1);
    assert!(emit(&mut prior, &mut pseq, P::Idle, None, None, 0).program_done);
    prior.apply(SessionEvent::TakeCompletion, t1);
    prior.apply(SessionEvent::Unread { viewed: false }, t1);
    let prior_unread = prior.status().unread_since;
    assert_eq!(prior_unread, Some(t1.wall_millis));
    prior.apply(
        SessionEvent::Agent {
            event: A::ProgramSessionFinished {
                session_id: "ses_abc123".into(),
                ended_at_ms: t1.wall_millis + 10,
                superseded: false,
            },
            live: true,
        },
        t1,
    );
    // Turn 2 completes optimistically and records Turn 2's Unread at t2, then fails late.
    emit(&mut prior, &mut pseq, P::Working, None, None, 1);
    assert!(emit(&mut prior, &mut pseq, P::Idle, None, None, 0).program_done);
    prior.apply(SessionEvent::TakeCompletion, t2);
    prior.apply(SessionEvent::Unread { viewed: false }, t2);
    assert_eq!(prior.status().unread_since, Some(t2.wall_millis));
    let late_turn2 = prior.apply(
        SessionEvent::Agent {
            event: A::ProgramSessionOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                session_id: "ses_abc123".into(),
                ended_at_ms: t2.wall_millis + 50,
                superseded: false,
            },
            live: true,
        },
        t2,
    );
    assert!(
        late_turn2.revert_unread,
        "Turn 2's optimistic completion must trigger CAS rollback in the manager"
    );
    assert_eq!(
        prior.status().unread_since,
        prior_unread,
        "Turn 2's rollback must restore Turn 1's unread_since rather than wiping it"
    );
    assert_eq!(
        prior.status().observation.outcome,
        Some(TurnOutcome::Failed)
    );

    // 8. Delayed Turn 1 failure arriving AFTER Turn 2 is already Working reverts Turn 1's
    // optimistic unread without poisoning Turn 2's subsequent Completed outcome.
    let mut adj = make_model();
    let mut aseq = 0u64;
    let emit_at =
        |model: &mut SessionModel, seq: &mut u64, state: P, child_id: Option<&str>, at: Now| {
            *seq += 1;
            let root = Report {
                id: String::new(),
                state,
                app: Some("opencode".into()),
                kind: None,
                progress: None,
                title: None,
                message: None,
            };
            let eff = model.apply(
                SessionEvent::ProgramStatus {
                    seq: *seq,
                    subseq: 0,
                    update: Box::new(Update {
                        current: Some(root.clone()),
                        root: Some(root.clone()),
                        active_children: Vec::new(),
                        trigger: Some(root.clone()),
                    }),
                    live: true,
                },
                at,
            );
            if let Some(cid) = child_id {
                *seq += 1;
                let child = Report {
                    id: cid.into(),
                    state,
                    app: Some("opencode".into()),
                    kind: None,
                    progress: None,
                    title: None,
                    message: None,
                };
                model.apply(
                    SessionEvent::ProgramStatus {
                        seq: *seq,
                        subseq: 0,
                        update: Box::new(Update {
                            current: Some(child.clone()),
                            root: Some(root),
                            active_children: vec![ChildCount {
                                app: Some("opencode".into()),
                                active: 1,
                            }],
                            trigger: Some(child),
                        }),
                        live: true,
                    },
                    at,
                );
            }
            eff
        };
    let turn1_at = t1;
    let turn2_at = t2;
    emit_at(&mut adj, &mut aseq, P::Working, Some("ses_a"), turn1_at);
    assert!(emit_at(&mut adj, &mut aseq, P::Idle, None, turn1_at).program_done);
    adj.apply(SessionEvent::TakeCompletion, turn1_at);
    adj.apply(SessionEvent::Unread { viewed: false }, turn1_at);
    assert_eq!(adj.status().unread_since, Some(turn1_at.wall_millis));
    // Turn 2 starts immediately before Turn 1's DB failure is polled.
    emit_at(&mut adj, &mut aseq, P::Working, Some("ses_a"), turn2_at);
    let delayed = adj.apply(
        SessionEvent::Agent {
            event: A::ProgramSessionOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                session_id: "ses_a".into(),
                ended_at_ms: turn1_at.wall_millis + 50,
                superseded: true,
            },
            live: true,
        },
        turn2_at,
    );
    assert!(
        delayed.revert_unread,
        "delayed Turn 1 failure must revert Turn 1's optimistic unread"
    );
    assert_eq!(adj.status().unread_since, None);
    assert_eq!(adj.status().observation.activity, Activity::Working);
    // Turn 2 itself succeeds cleanly -> Completed (not poisoned by Turn 1's delayed failure!).
    let turn2_done = emit_at(&mut adj, &mut aseq, P::Idle, None, turn2_at);
    assert!(turn2_done.program_done);
    assert_eq!(
        adj.status().observation.outcome,
        Some(TurnOutcome::Completed)
    );
    assert_eq!(adj.status().failed_since, None);

    // 9. Turn 1 failed, Turn 2 in the SAME ses_a went working -> idle cleanly, THEN Turn 1's
    // delayed failure (superseded=true or ended_at_ms < Turn 1 idle) arrives while idle:
    // Turn 2 must stay Completed with its unread badge intact; delayed Turn 1 success must
    // also never finalize Turn 2's idle_turn prematurely.
    let mut same_ses = make_model();
    let mut sseq = 0u64;
    let t1_idle_at = Now {
        monotonic: start.monotonic + Duration::from_secs(1),
        wall_millis: start.wall_millis + 1_000,
    };
    let t2_work_at = Now {
        monotonic: start.monotonic + Duration::from_secs(2),
        wall_millis: start.wall_millis + 2_000,
    };
    let t2_idle_at = Now {
        monotonic: start.monotonic + Duration::from_secs(3),
        wall_millis: start.wall_millis + 3_000,
    };
    emit_at(&mut same_ses, &mut sseq, P::Working, Some("ses_a"), start);
    assert!(emit_at(&mut same_ses, &mut sseq, P::Idle, None, t1_idle_at).program_done);
    same_ses.apply(SessionEvent::TakeCompletion, t1_idle_at);
    same_ses.apply(SessionEvent::Unread { viewed: false }, t1_idle_at);
    // Turn 2 runs and goes idle cleanly before Turn 1's DB failure event is consumed.
    emit_at(
        &mut same_ses,
        &mut sseq,
        P::Working,
        Some("ses_a"),
        t2_work_at,
    );
    assert!(emit_at(&mut same_ses, &mut sseq, P::Idle, None, t2_idle_at).program_done);
    same_ses.apply(SessionEvent::TakeCompletion, t2_idle_at);
    same_ses.apply(SessionEvent::Unread { viewed: false }, t2_idle_at);
    assert_eq!(same_ses.status().unread_since, Some(t2_idle_at.wall_millis));

    for (superseded, ended_at_ms) in [
        (true, t1_idle_at.wall_millis + 50),
        (false, t1_idle_at.wall_millis - 10),
    ] {
        let stale_fail = same_ses.apply(
            SessionEvent::Agent {
                event: A::ProgramSessionOutcome {
                    outcome: TurnOutcome::Failed,
                    policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                    session_id: "ses_a".into(),
                    ended_at_ms,
                    superseded,
                },
                live: true,
            },
            t2_idle_at,
        );
        assert!(
            !stale_fail.revert_unread,
            "delayed Turn 1 failure (superseded={superseded}) must not revert Turn 2's unread"
        );
        assert_eq!(
            same_ses.status().observation.outcome,
            Some(TurnOutcome::Completed)
        );
        assert_eq!(same_ses.status().unread_since, Some(t2_idle_at.wall_millis));
    }
    let stale_fin = same_ses.apply(
        SessionEvent::Agent {
            event: A::ProgramSessionFinished {
                session_id: "ses_a".into(),
                ended_at_ms: t1_idle_at.wall_millis - 10,
                superseded: true,
            },
            live: true,
        },
        t2_idle_at,
    );
    assert!(!stale_fin.clear_optimistic_completion);

    // 10. Foreign DB failure when this TUI has ONLY EVER shown root idle must be ignored.
    let mut idle_only = make_model();
    let mut iseq = 0u64;
    emit_at(&mut idle_only, &mut iseq, P::Idle, None, start);
    let foreign = idle_only.apply(
        SessionEvent::Agent {
            event: A::ProgramSessionOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                session_id: "ses_visited".into(),
                ended_at_ms: start.wall_millis + 500,
                superseded: false,
            },
            live: true,
        },
        start,
    );
    assert!(foreign.publication.is_none());
    assert_eq!(idle_only.status().observation.outcome, None);
    assert_eq!(idle_only.status().failed_since, None);

    // 11. Viewing the chat between idle-inferred Completed and the matching DB Failed /
    // Interrupted outcome clears unread and drops rollback state while still correcting
    // the displayed outcome (and never restoring unread).
    for expected in [TurnOutcome::Failed, TurnOutcome::Interrupted] {
        let mut viewed = make_model();
        let mut vseq = 0u64;
        emit_at(&mut viewed, &mut vseq, P::Idle, None, start);
        emit_at(&mut viewed, &mut vseq, P::Working, Some("ses_v"), start);
        assert!(emit_at(&mut viewed, &mut vseq, P::Idle, None, start).program_done);
        viewed.apply(SessionEvent::TakeCompletion, start);
        viewed.apply(SessionEvent::Unread { viewed: false }, start);
        assert!(viewed.status().unread_since.is_some());
        let view_eff = viewed.apply(SessionEvent::Viewed, start);
        assert!(view_eff.clear_optimistic_completion);
        assert_eq!(viewed.status().unread_since, None);

        let late = viewed.apply(
            SessionEvent::Agent {
                event: A::ProgramSessionOutcome {
                    outcome: expected,
                    policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                    session_id: "ses_v".into(),
                    ended_at_ms: start.wall_millis + 25,
                    superseded: false,
                },
                live: true,
            },
            start,
        );
        assert!(
            !late.revert_unread,
            "viewed turn must not trigger unread rollback on late {expected:?}"
        );
        assert_eq!(viewed.status().unread_since, None);
        assert_eq!(viewed.status().observation.outcome, Some(expected));
        assert_eq!(
            viewed.status().failed_since.is_some(),
            expected == TurnOutcome::Failed
        );
    }

    // 12. DB outcome leading the PTY stream stays buffered while idle, reconciles when the
    // matching working/child/idle sequence arrives, and never lets a foreign session bleed
    // into another session's turn or overwrite an explicit completion.
    let mut lead = make_model();
    let mut lseq = 0u64;
    emit_at(&mut lead, &mut lseq, P::Idle, None, start);
    let early_db = lead.apply(
        SessionEvent::Agent {
            event: A::ProgramSessionOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                session_id: "ses_lead".into(),
                ended_at_ms: start.wall_millis + 10,
                superseded: false,
            },
            live: true,
        },
        start,
    );
    assert!(early_db.publication.is_none());
    assert_eq!(lead.status().observation.outcome, None);
    emit_at(&mut lead, &mut lseq, P::Working, Some("ses_lead"), start);
    let settled = emit_at(&mut lead, &mut lseq, P::Idle, None, start);
    assert!(!settled.program_done);
    assert_eq!(lead.status().observation.outcome, Some(TurnOutcome::Failed));
    assert!(lead.status().failed_since.is_some());
    assert!(
        !lead
            .apply(SessionEvent::TakeCompletion, start)
            .completion_consumed
    );

    // Foreign buffered failure on ses_foreign does NOT poison a turn on ses_other.
    let mut foreign_guard = make_model();
    let mut fseq = 0u64;
    emit_at(&mut foreign_guard, &mut fseq, P::Idle, None, start);
    foreign_guard.apply(
        SessionEvent::Agent {
            event: A::ProgramSessionOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                session_id: "ses_foreign".into(),
                ended_at_ms: start.wall_millis + 10,
                superseded: false,
            },
            live: true,
        },
        start,
    );
    emit_at(
        &mut foreign_guard,
        &mut fseq,
        P::Working,
        Some("ses_other"),
        start,
    );
    let other_done = emit_at(&mut foreign_guard, &mut fseq, P::Idle, None, start);
    assert!(other_done.program_done);
    assert_eq!(
        foreign_guard.status().observation.outcome,
        Some(TurnOutcome::Completed)
    );
    // Explicit ProgramSessionFinished protects Completed against a later idle-only DB Failed.
    let confirmed = foreign_guard.apply(
        SessionEvent::Agent {
            event: A::ProgramSessionFinished {
                session_id: "ses_other".into(),
                ended_at_ms: start.wall_millis + 20,
                superseded: false,
            },
            live: true,
        },
        start,
    );
    assert!(confirmed.clear_optimistic_completion);
    let after_confirm = foreign_guard.apply(
        SessionEvent::Agent {
            event: A::ProgramSessionOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                session_id: "ses_other".into(),
                ended_at_ms: start.wall_millis + 30,
                superseded: false,
            },
            live: true,
        },
        start,
    );
    assert!(after_confirm.publication.is_none());
    assert_eq!(
        foreign_guard.status().observation.outcome,
        Some(TurnOutcome::Completed)
    );

    // 13. Cancelled turn consumes its matching DB Interrupted confirmation instead of leaking
    // into the next successful turn, while genuinely early evidence for a newer turn still
    // reconciles cleanly.
    let mut cancelled_next = make_model();
    let mut cseq = 0u64;
    let c1_idle = Now {
        monotonic: start.monotonic + Duration::from_secs(1),
        wall_millis: start.wall_millis + 1_000,
    };
    let c2_at = Now {
        monotonic: start.monotonic + Duration::from_secs(2),
        wall_millis: start.wall_millis + 2_000,
    };
    emit_at(&mut cancelled_next, &mut cseq, P::Idle, None, start);
    emit_at(
        &mut cancelled_next,
        &mut cseq,
        P::Working,
        Some("ses_c"),
        start,
    );
    cancelled_next.apply(
        SessionEvent::Input {
            class: None,
            submitted: false,
            cancel: true,
        },
        c1_idle,
    );
    assert!(!emit_at(&mut cancelled_next, &mut cseq, P::Idle, None, c1_idle).program_done);
    assert_eq!(
        cancelled_next.status().observation.outcome,
        Some(TurnOutcome::Interrupted)
    );
    // Matching DB confirmation for the cancelled turn arrives after root idle -> consumed.
    cancelled_next.apply(
        SessionEvent::Agent {
            event: A::ProgramSessionOutcome {
                outcome: TurnOutcome::Interrupted,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                session_id: "ses_c".into(),
                ended_at_ms: c1_idle.wall_millis + 25,
                superseded: false,
            },
            live: true,
        },
        c1_idle,
    );
    // Next successful turn on the same session must complete cleanly with program_done = true.
    emit_at(
        &mut cancelled_next,
        &mut cseq,
        P::Working,
        Some("ses_c"),
        c2_at,
    );
    let c2_done = emit_at(&mut cancelled_next, &mut cseq, P::Idle, None, c2_at);
    assert!(
        c2_done.program_done,
        "Turn 2 after Ctrl+C must complete cleanly rather than consuming Turn 1's DB confirmation"
    );
    assert_eq!(
        cancelled_next.status().observation.outcome,
        Some(TurnOutcome::Completed)
    );

    // Genuinely early Failed for Turn 2 arriving after Turn 1's cancel (before Turn 2's PTY)
    // is NOT swallowed by program_cancelled_sessions and still reconciles to Failed.
    let mut cancel_then_early_fail = make_model();
    let mut cfseq = 0u64;
    emit_at(
        &mut cancel_then_early_fail,
        &mut cfseq,
        P::Idle,
        None,
        start,
    );
    emit_at(
        &mut cancel_then_early_fail,
        &mut cfseq,
        P::Working,
        Some("ses_c"),
        start,
    );
    cancel_then_early_fail.apply(
        SessionEvent::Input {
            class: None,
            submitted: false,
            cancel: true,
        },
        c1_idle,
    );
    emit_at(
        &mut cancel_then_early_fail,
        &mut cfseq,
        P::Idle,
        None,
        c1_idle,
    );
    cancel_then_early_fail.apply(
        SessionEvent::Agent {
            event: A::ProgramSessionOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                session_id: "ses_c".into(),
                ended_at_ms: c1_idle.wall_millis + 50,
                superseded: false,
            },
            live: true,
        },
        c1_idle,
    );
    emit_at(
        &mut cancel_then_early_fail,
        &mut cfseq,
        P::Working,
        Some("ses_c"),
        c2_at,
    );
    let early_fail_done = emit_at(
        &mut cancel_then_early_fail,
        &mut cfseq,
        P::Idle,
        None,
        c2_at,
    );
    assert!(!early_fail_done.program_done);
    assert_eq!(
        cancel_then_early_fail.status().observation.outcome,
        Some(TurnOutcome::Failed)
    );

    // 14. DB outcome arriving strictly BETWEEN root Working (unknown membership) and the
    // first child id=ses_b record is buffered until the child identifies the participant:
    // foreign ses_a leaves ses_b Completed, while matching ses_b settles Failed.
    for (db_session, expected_outcome, expect_done) in [
        ("ses_a", TurnOutcome::Completed, true),
        ("ses_b", TurnOutcome::Failed, false),
    ] {
        let mut between = make_model();
        let mut bseq = 0u64;
        emit_at(&mut between, &mut bseq, P::Idle, None, start);
        // Root Working ONLY (no child yet -> program_turn_sessions is empty).
        emit_at(&mut between, &mut bseq, P::Working, None, start);
        between.apply(
            SessionEvent::Agent {
                event: A::ProgramSessionOutcome {
                    outcome: TurnOutcome::Failed,
                    policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                    session_id: db_session.into(),
                    ended_at_ms: start.wall_millis + 10,
                    superseded: false,
                },
                live: true,
            },
            start,
        );
        // Now child ses_b arrives and identifies the active participant.
        bseq += 1;
        let child_b = Report {
            id: "ses_b".into(),
            state: P::Working,
            app: Some("opencode".into()),
            kind: None,
            progress: None,
            title: None,
            message: None,
        };
        let root_w = Report {
            id: String::new(),
            state: P::Working,
            app: Some("opencode".into()),
            kind: None,
            progress: None,
            title: None,
            message: None,
        };
        between.apply(
            SessionEvent::ProgramStatus {
                seq: bseq,
                subseq: 0,
                update: Box::new(Update {
                    current: Some(child_b.clone()),
                    root: Some(root_w),
                    active_children: vec![ChildCount {
                        app: Some("opencode".into()),
                        active: 1,
                    }],
                    trigger: Some(child_b),
                }),
                live: true,
            },
            start,
        );
        let done = emit_at(&mut between, &mut bseq, P::Idle, None, start);
        assert_eq!(done.program_done, expect_done, "db_session={db_session}");
        assert_eq!(
            between.status().observation.outcome,
            Some(expected_outcome),
            "db_session={db_session}"
        );
    }

    // 14b. Same-session Turn 2 failure arriving between root Working and child id=ses_a after
    // a completed Turn 1 (whose unread badge U1 still stands) must preserve U1, settle Turn 2
    // as Failed, and not fire a second completion.
    let mut second_turn = make_model();
    let mut stseq = 0u64;
    let st1_at = Now {
        monotonic: start.monotonic + Duration::from_secs(1),
        wall_millis: start.wall_millis + 1_000,
    };
    let st2_at = Now {
        monotonic: start.monotonic + Duration::from_secs(2),
        wall_millis: start.wall_millis + 2_000,
    };
    emit_at(&mut second_turn, &mut stseq, P::Idle, None, start);
    emit_at(
        &mut second_turn,
        &mut stseq,
        P::Working,
        Some("ses_a"),
        st1_at,
    );
    // Turn 1's DB succeeded poll can arrive ahead of Turn 1's root idle, leaving idle_turn
    // active once root idle runs and records Turn 1's unread U1.
    second_turn.apply(
        SessionEvent::Agent {
            event: A::ProgramSessionFinished {
                session_id: "ses_a".into(),
                ended_at_ms: st1_at.wall_millis + 10,
                superseded: false,
            },
            live: true,
        },
        st1_at,
    );
    assert!(emit_at(&mut second_turn, &mut stseq, P::Idle, None, st1_at).program_done);
    assert!(
        second_turn
            .apply(SessionEvent::TakeCompletion, st1_at)
            .completion_consumed
    );
    second_turn.apply(SessionEvent::Unread { viewed: false }, st1_at);
    let turn1_unread = second_turn.status().unread_since;
    assert_eq!(turn1_unread, Some(st1_at.wall_millis));

    // Turn 2 root Working arrives BEFORE child ses_a; non-superseded DB Failed for ses_a
    // arrives with ended_at_ms after Turn 2 started.
    emit_at(&mut second_turn, &mut stseq, P::Working, None, st2_at);
    let mid = second_turn.apply(
        SessionEvent::Agent {
            event: A::ProgramSessionOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                session_id: "ses_a".into(),
                ended_at_ms: st2_at.wall_millis + 20,
                superseded: false,
            },
            live: true,
        },
        st2_at,
    );
    assert!(
        !mid.revert_unread,
        "Turn 2 failure must not roll back Turn 1's unread badge"
    );
    assert_eq!(second_turn.status().unread_since, turn1_unread);

    // Child ses_a arrives, then root idle settles Turn 2 as Failed with Turn 1's unread intact.
    stseq += 1;
    let child_a = Report {
        id: "ses_a".into(),
        state: P::Working,
        app: Some("opencode".into()),
        kind: None,
        progress: None,
        title: None,
        message: None,
    };
    let root_a = Report {
        id: String::new(),
        state: P::Working,
        app: Some("opencode".into()),
        kind: None,
        progress: None,
        title: None,
        message: None,
    };
    second_turn.apply(
        SessionEvent::ProgramStatus {
            seq: stseq,
            subseq: 0,
            update: Box::new(Update {
                current: Some(child_a.clone()),
                root: Some(root_a),
                active_children: vec![ChildCount {
                    app: Some("opencode".into()),
                    active: 1,
                }],
                trigger: Some(child_a),
            }),
            live: true,
        },
        st2_at,
    );
    let st2_done = emit_at(&mut second_turn, &mut stseq, P::Idle, None, st2_at);
    assert!(!st2_done.program_done);
    assert_eq!(
        second_turn.status().observation.outcome,
        Some(TurnOutcome::Failed)
    );
    assert!(second_turn.status().failed_since.is_some());
    assert_eq!(
        second_turn.status().unread_since,
        turn1_unread,
        "Turn 1's unread timestamp must stay preserved"
    );
    assert!(
        !second_turn
            .apply(SessionEvent::TakeCompletion, st2_at)
            .completion_consumed,
        "failed Turn 2 must not arm a second completion"
    );
}
