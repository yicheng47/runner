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
            current: Some(foreign_child),
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
        model.apply(SessionEvent::Input { class, submitted }, at);
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
                update: Box::new(Update { current, trigger }),
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

    // 7. Non-Codex authoritative runtimes (Antigravity, hooks-off Claude Code and pi)
    // dismiss OSC 7501 error on trailing idle to plain Idle (outcome == None), whereas
    // Codex reconciles transcript/OSC errors and preserves Failed until new work.
    for (runtime, app, status_hooks) in [
        (crate::model::Runtime::Antigravity, "agy", true),
        (crate::model::Runtime::ClaudeCode, "claude", false),
        (crate::model::Runtime::Pi, "pi", false),
    ] {
        let semantics = crate::runtimes::adapter(runtime).status_semantics();
        assert!(!semantics.program_status_transcript_outcomes, "{runtime:?}");
        let mut other = SessionModel::default();
        other.set_program_status_authority(semantics.program_status_authoritative(status_hooks));
        other.set_program_status_root_app(semantics.program_status_root_app);
        other.set_program_status_transcript_outcomes(semantics.program_status_transcript_outcomes);
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
            None,
            "{runtime:?}: error -> idle must return plain Idle"
        );
        assert_eq!(other.status().failed_since, None, "{runtime:?}");
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
                            event: A::ProgramTurnOutcome { outcome },
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
