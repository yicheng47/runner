use super::*;
use runner_core::protocol::status::{AgentObservation, HumanInteraction};

fn presentation(
    kind: StatusKind,
    outcome: Option<TurnOutcome>,
    waiting_since: Option<i64>,
) -> StatusPresentation {
    StatusPresentation {
        kind,
        estimated: false,
        exit_code: None,
        outcome,
        waiting_since,
        detail: None,
        message: None,
        program_status: false,
        background_tasks: 0,
        interrupted_since: None,
    }
}

#[test]
fn elapsed_time_uses_seconds_then_minutes_then_hours() {
    assert_eq!(format_elapsed(0), "0s");
    assert_eq!(format_elapsed(59_000), "59s");
    assert_eq!(format_elapsed(60_000), "1m");
    assert_eq!(format_elapsed(3_599_000), "59m");
    assert_eq!(format_elapsed(3_600_000), "1h 0m");
    assert_eq!(format_elapsed(4_320_000), "1h 12m");
    assert_eq!(format_elapsed(86_400_000), "24h 0m");
}

#[test]
fn program_status_is_exact_and_blocked_message_uses_the_existing_detail_label() {
    let mut status = AgentStatus {
        lifecycle: Lifecycle::Running,
        observation: AgentObservation {
            activity: Activity::Working,
            source: ObservationSource::Baseline,
            program_status: Some(runner_core::protocol::status::ProgramStatusEvidence {
                message: Some("Allow this command?".into()),
                background_tasks: 0,
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let working = StatusPresentation::new(&status);
    assert!(!working.estimated);
    assert!(working.program_status);
    status.observation.interactions.push(HumanInteraction {
        id: "program-status:1".into(),
        reason: WaitReason::Approval,
        owners: Vec::new(),
        since: 0,
    });
    let blocked = StatusPresentation::new(&status);
    assert_eq!(
        blocked.visible_short_label_at(480., 120_000).as_deref(),
        Some("Approval needed · Allow this command?")
    );
    assert!(blocked.tooltip_at(120_000).contains("Allow this command?"));
    assert!(blocked.tooltip_at(120_000).ends_with("2m"));
}

#[test]
fn idle_tooltip_distinguishes_only_an_interrupted_outcome() {
    for outcome in [
        None,
        Some(TurnOutcome::Completed),
        Some(TurnOutcome::Failed),
    ] {
        assert_eq!(
            presentation(StatusKind::Idle, outcome, None).tooltip_at(0),
            "Idle"
        );
    }
    let interrupted = presentation(StatusKind::Idle, Some(TurnOutcome::Interrupted), None);
    assert_eq!(
        interrupted.tooltip_at(0),
        "Idle · Last response interrupted"
    );
    assert_eq!(interrupted.short_label_at(0).text(), "Idle · Interrupted");
    assert_eq!(
        StatusPresentation {
            estimated: true,
            ..interrupted
        }
        .tooltip_at(0),
        "Idle · estimated from terminal activity"
    );
}

#[test]
fn an_interrupted_member_reads_interrupted_in_place_of_idle_until_it_works() {
    let mut status = AgentStatus {
        lifecycle: Lifecycle::Running,
        observation: AgentObservation {
            activity: Activity::Idle,
            source: ObservationSource::Hook,
            ..Default::default()
        },
        interrupted_since: Some(1_767_225_600_000),
        ..Default::default()
    };
    let interrupted = StatusPresentation::new(&status);
    assert_eq!(interrupted.kind, StatusKind::Interrupted);
    assert_eq!(interrupted.label(), "Interrupted");
    assert!(interrupted.warns());
    let time = chrono::Local
        .timestamp_millis_opt(1_767_225_600_000)
        .unwrap()
        .format("%-I:%M %p");
    assert_eq!(
        interrupted.tooltip_at(0),
        format!("Interrupted by a Runner restart at {time}; message it to continue.")
    );
    status.observation.activity = Activity::Working;
    assert_eq!(StatusPresentation::new(&status).kind, StatusKind::Working);
    status.observation.activity = Activity::Idle;
    status.lifecycle = Lifecycle::Stopped;
    assert_eq!(StatusPresentation::new(&status).kind, StatusKind::Stopped);
}

#[test]
fn wait_tooltips_and_card_subtitles_use_the_oldest_interaction() {
    let mut status = AgentStatus {
        lifecycle: Lifecycle::Running,
        observation: AgentObservation {
            activity: Activity::Working,
            source: ObservationSource::Hook,
            interactions: vec![
                HumanInteraction {
                    id: "oldest".into(),
                    reason: WaitReason::Approval,
                    owners: vec![],
                    since: 0,
                },
                HumanInteraction {
                    id: "newer".into(),
                    reason: WaitReason::Answer,
                    owners: vec![],
                    since: 30_000,
                },
            ],
            ..Default::default()
        },
        ..Default::default()
    };
    let approval = StatusPresentation::new(&status);
    for (now, elapsed) in [
        (0, "0s"),
        (59_000, "59s"),
        (60_000, "1m"),
        (3_599_000, "59m"),
        (3_600_000, "1h 0m"),
        (4_320_000, "1h 12m"),
        (86_400_000, "24h 0m"),
    ] {
        assert_eq!(
            approval.tooltip_at(now),
            format!("Waiting for you to approve a command or plan · {elapsed}")
        );
    }
    assert_eq!(approval.short_label_at(59_999).text(), "Approval needed");
    assert_eq!(
        approval.short_label_at(120_000).text(),
        "Approval needed · 2m"
    );

    status.observation.interactions[0].reason = WaitReason::Answer;
    let answer = StatusPresentation::new(&status);
    assert_eq!(answer.tooltip_at(45_000), "Waiting for your answer · 45s");
    assert_eq!(
        answer.short_label_at(4_320_000).text(),
        "Answer needed · 1h 12m"
    );

    status.observation.interactions[0].reason = WaitReason::Unknown;
    let needs_you = StatusPresentation::new(&status);
    assert_eq!(
        needs_you.tooltip_at(120_000),
        "Waiting for your input in the terminal · 2m"
    );
    assert_eq!(needs_you.short_label_at(120_000).text(), "Needs you · 2m");
}

#[test]
fn working_detail_uses_shared_short_labels_and_full_tooltips() {
    let plain = presentation(StatusKind::Working, None, None);
    assert_eq!(plain.tooltip_at(0), "Working");
    assert_eq!(plain.short_label_at(0).text(), "Working");

    for (detail, label) in [
        (WorkDetail::UsingTools, "Using tools"),
        (WorkDetail::CompactingContext, "Compacting context"),
    ] {
        let detailed = StatusPresentation {
            detail: Some(detail),
            ..plain.clone()
        };
        assert_eq!(detailed.label(), "Working");
        assert_eq!(detailed.tooltip_at(0), format!("Working · {label}"));
        assert_eq!(
            detailed.short_label_at(0).text(),
            format!("Working · {label}")
        );
        let estimated = StatusPresentation {
            estimated: true,
            ..detailed
        };
        assert_eq!(
            estimated.tooltip_at(0),
            "Working · estimated from terminal activity"
        );
        assert_eq!(estimated.short_label_at(0).text(), "Working");
    }
}

#[test]
fn header_uses_the_shared_short_label_and_drops_detail_first() {
    let working = StatusPresentation {
        detail: Some(WorkDetail::CompactingContext),
        ..presentation(StatusKind::Working, None, None)
    };
    assert_eq!(
        working.visible_short_label_at(560., 0),
        Some("Working · Compacting context".into())
    );
    assert_eq!(
        working.visible_short_label_at(559., 0),
        Some("Working".into())
    );
    assert_eq!(working.visible_short_label_at(479., 0), None);

    let approval = presentation(StatusKind::Approval, None, Some(0));
    assert_eq!(
        approval.visible_short_label_at(400., 120_000),
        Some("Approval needed · 2m".into())
    );
    assert_eq!(
        approval.visible_short_label_at(399., 120_000),
        Some("Approval needed".into())
    );
    assert_eq!(approval.visible_short_label_at(319., 120_000), None);
}

#[test]
fn idle_shares_one_label_and_narrow_labels_preserve_attention() {
    let mut status = AgentStatus {
        lifecycle: Lifecycle::Running,
        observation: AgentObservation {
            activity: Activity::Idle,
            source: ObservationSource::Baseline,
            ..Default::default()
        },
        ..Default::default()
    };
    let presentation = StatusPresentation::new(&status);
    assert_eq!(presentation.kind, StatusKind::Idle);
    assert_eq!(presentation.label(), "Idle");
    assert!(presentation.estimated);
    assert_eq!(
        presentation.tooltip(),
        "Idle · estimated from terminal activity"
    );
    assert!(!presentation.shows_label(360.));
    status.observation.activity = Activity::Ready;
    status.observation.source = ObservationSource::Hook;
    let confirmed = StatusPresentation::new(&status);
    assert_eq!(confirmed.kind, presentation.kind);
    assert_eq!(confirmed.label(), presentation.label());
    assert!(!confirmed.estimated);
    assert_eq!(confirmed.tooltip(), "Idle");
    let approval = StatusPresentation {
        kind: StatusKind::Approval,
        estimated: false,
        exit_code: None,
        outcome: None,
        waiting_since: None,
        detail: None,
        message: None,
        program_status: false,
        background_tasks: 0,
        interrupted_since: None,
    };
    assert!(approval.shows_label(360.));
    assert!(!approval.shows_label(296.));
}
