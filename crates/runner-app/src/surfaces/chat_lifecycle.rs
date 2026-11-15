use std::time::Duration;

use runner_core::protocol::model::SessionStatus;

pub(crate) const TRANSITION_MIN_VISIBLE: Duration = Duration::from_secs(1);
pub(crate) const TRANSITION_IDLE: Duration = Duration::from_millis(400);
pub(crate) const TRANSITION_HARD_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransitionKind {
    Starting,
    Resuming,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PaneOverlayState {
    Archiving,
    Resuming,
    Starting,
    Ended {
        status: SessionStatus,
        resumable: bool,
        exit_code: Option<i32>,
    },
    None,
}

pub(crate) fn resolve_pane_overlay(
    archiving: bool,
    transition: Option<TransitionKind>,
    status: SessionStatus,
    resumable: bool,
    exit_code: Option<i32>,
) -> PaneOverlayState {
    if archiving {
        PaneOverlayState::Archiving
    } else if transition == Some(TransitionKind::Resuming) {
        PaneOverlayState::Resuming
    } else if transition == Some(TransitionKind::Starting) {
        PaneOverlayState::Starting
    } else if status != SessionStatus::Running {
        PaneOverlayState::Ended {
            status,
            resumable,
            exit_code,
        }
    } else {
        PaneOverlayState::None
    }
}

pub(crate) fn take_visible_drawer_launch_claim<E>(
    visible: bool,
    status: SessionStatus,
    take: impl FnOnce() -> Result<bool, E>,
) -> Result<bool, E> {
    if !visible || status == SessionStatus::Running {
        return Ok(false);
    }
    take()
}

pub(crate) fn ended_subtitle(
    status: SessionStatus,
    resumable: bool,
    exit_code: Option<i32>,
) -> String {
    if !resumable {
        return crate::i18n::text("ui.the_pty_is_closed_resume_to_start_a_fresh_agent_process_there_s_no_saved_conversation_to_p").into();
    }
    if status == SessionStatus::Crashed {
        return exit_code.map_or_else(
            || crate::i18n::text("ui.the_pty_exited_unexpectedly_resume_to_start_a_fresh_process_the_prior_agent_conversation_i").into(),
            |code| crate::i18n::tf("format.the_pty_exited_with_code_resume_to_start_a_fresh_process_the_prior_agent_conve", &[("code", (code).to_string())]),
        );
    }
    crate::i18n::text("ui.the_pty_is_closed_but_the_conversation_is_preserved_resume_to_pick_up_where_you_left_off")
        .into()
}

/// The pill drops on the first painted frame. The output-idle backstop covers
/// non-TUI processes that emit non-painting output and go quiet, while keeping
/// the pill up across both cold-start silence before any PTY byte arrives and
/// TUI startup silence after taking control of the terminal (`tui_ready_seen`)
/// before painting the first frame.
pub(crate) fn transition_should_settle(
    _kind: TransitionKind,
    elapsed: Duration,
    first_paint_seen: bool,
    tui_ready_seen: bool,
    output_seen: bool,
    output_idle_for: Option<Duration>,
) -> bool {
    first_paint_seen
        || elapsed >= TRANSITION_HARD_TIMEOUT
        || (output_seen
            && !tui_ready_seen
            && elapsed >= TRANSITION_MIN_VISIBLE
            && output_idle_for.is_some_and(|idle| idle >= TRANSITION_IDLE))
}

pub(crate) fn spawned_output_seq(payload: &serde_json::Value) -> Option<u64> {
    payload
        .get("output_seq")
        .and_then(serde_json::Value::as_u64)
}

pub(crate) fn evaluate_transition_settle(
    kind: TransitionKind,
    baseline_seq: u64,
    elapsed: Duration,
    activity: Option<runner_terminal::terminal::TerminalOutputActivity>,
    now: std::time::Instant,
) -> bool {
    let output_seen = activity.is_some_and(|activity| activity.last_output_seq > baseline_seq);
    let first_paint_seen = activity.is_some_and(|activity| activity.last_paint_seq > baseline_seq);
    let tui_ready_seen = activity.is_some_and(|activity| activity.tui_ready_seq > baseline_seq);
    let output_idle_for = output_seen
        .then(|| activity.and_then(|activity| activity.last_output_at))
        .flatten()
        .map(|at| now.saturating_duration_since(at));
    transition_should_settle(
        kind,
        elapsed,
        first_paint_seen,
        tui_ready_seen,
        output_seen,
        output_idle_for,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_precedence_matches_the_react_surface() {
        assert_eq!(
            resolve_pane_overlay(
                true,
                Some(TransitionKind::Resuming),
                SessionStatus::Crashed,
                true,
                Some(1),
            ),
            PaneOverlayState::Archiving
        );
        assert_eq!(
            resolve_pane_overlay(
                false,
                Some(TransitionKind::Resuming),
                SessionStatus::Stopped,
                true,
                None,
            ),
            PaneOverlayState::Resuming
        );
        assert_eq!(
            resolve_pane_overlay(
                false,
                Some(TransitionKind::Starting),
                SessionStatus::Running,
                true,
                None,
            ),
            PaneOverlayState::Starting
        );
        assert_eq!(
            resolve_pane_overlay(false, None, SessionStatus::Stopped, false, None),
            PaneOverlayState::Ended {
                status: SessionStatus::Stopped,
                resumable: false,
                exit_code: None,
            }
        );
    }

    #[test]
    fn drawer_shells_resume_only_when_visible_and_claimed() {
        let mut takes = 0;
        assert!(
            take_visible_drawer_launch_claim(true, SessionStatus::Stopped, || {
                takes += 1;
                Ok::<_, ()>(true)
            })
            .unwrap()
        );
        assert_eq!(takes, 1);

        assert!(!take_visible_drawer_launch_claim(
            false,
            SessionStatus::Stopped,
            || -> Result<bool, ()> {
                panic!("a hidden drawer must leave its launch claim pending")
            }
        )
        .unwrap());
        assert!(!take_visible_drawer_launch_claim(
            true,
            SessionStatus::Running,
            || -> Result<bool, ()> {
                panic!("a running drawer shell must not take a launch claim")
            }
        )
        .unwrap());
    }

    #[test]
    fn stopped_drawer_shell_without_a_claim_keeps_the_exited_state() {
        assert!(
            !take_visible_drawer_launch_claim(true, SessionStatus::Stopped, || {
                Ok::<_, ()>(false)
            })
            .unwrap()
        );
        assert_eq!(
            resolve_pane_overlay(false, None, SessionStatus::Stopped, false, Some(0)),
            PaneOverlayState::Ended {
                status: SessionStatus::Stopped,
                resumable: false,
                exit_code: Some(0),
            }
        );
    }

    #[test]
    fn ended_copy_tracks_resume_and_crash_semantics() {
        assert!(ended_subtitle(SessionStatus::Stopped, false, None)
            .contains("there's no saved conversation"));
        assert!(ended_subtitle(SessionStatus::Crashed, true, Some(17))
            .starts_with("The PTY exited with code 17."));
        assert!(ended_subtitle(SessionStatus::Crashed, true, None)
            .starts_with("The PTY exited unexpectedly."));
        assert!(ended_subtitle(SessionStatus::Stopped, true, None)
            .starts_with("The PTY is closed, but the conversation is preserved."));
    }

    #[test]
    fn transition_settling_uses_first_paint_with_timeout_and_idle_backstops() {
        for kind in [TransitionKind::Starting, TransitionKind::Resuming] {
            assert!(transition_should_settle(
                kind,
                Duration::from_millis(20),
                true,
                true,
                true,
                Some(Duration::ZERO),
            ));
            assert!(!transition_should_settle(
                kind,
                Duration::from_secs(1),
                false,
                false,
                false,
                None,
            ));
            assert!(!transition_should_settle(
                kind,
                Duration::from_millis(2500),
                false,
                false,
                false,
                None,
            ));
            assert!(!transition_should_settle(
                kind,
                Duration::from_millis(900),
                false,
                false,
                true,
                Some(Duration::from_millis(500)),
            ));
            assert!(!transition_should_settle(
                kind,
                Duration::from_secs(1),
                false,
                false,
                true,
                Some(Duration::from_millis(250)),
            ));
            assert!(!transition_should_settle(
                kind,
                Duration::from_secs(1),
                false,
                false,
                true,
                None,
            ));
            assert!(transition_should_settle(
                kind,
                Duration::from_secs(1),
                false,
                false,
                true,
                Some(Duration::from_millis(400)),
            ));
            assert!(transition_should_settle(
                kind,
                TRANSITION_HARD_TIMEOUT,
                false,
                false,
                false,
                None,
            ));
        }
    }

    #[test]
    fn a_tui_that_took_the_screen_but_has_not_painted_keeps_the_pill() {
        // claude-code fullscreen: `?1049h` at ~250 ms, then silence while it
        // boots, first frame at 1–2 s. The idle backstop must not fire.
        for kind in [TransitionKind::Starting, TransitionKind::Resuming] {
            assert!(!transition_should_settle(
                kind,
                Duration::from_millis(1500),
                false,
                true,
                true,
                Some(Duration::from_millis(1200)),
            ));
            assert!(transition_should_settle(
                kind,
                Duration::from_millis(1800),
                true,
                true,
                true,
                Some(Duration::ZERO),
            ));
            assert!(transition_should_settle(
                kind,
                TRANSITION_HARD_TIMEOUT,
                false,
                true,
                true,
                Some(Duration::from_secs(9)),
            ));
        }
    }

    #[test]
    fn evaluate_transition_settle_tracks_baseline_and_ignores_resize_or_mode_only_resume_activity()
    {
        let now = std::time::Instant::now();
        let baseline = 50;

        // Restored pre-stop paint/TUI at baseline must keep Resuming up even if idle > 400ms.
        let restored = runner_terminal::terminal::TerminalOutputActivity {
            last_seq: 51,
            last_output_seq: 50,
            tui_ready_seq: 50,
            first_paint_seq: 50,
            last_paint_seq: 50,
            last_output_at: Some(now - Duration::from_secs(2)),
        };
        assert!(!evaluate_transition_settle(
            TransitionKind::Resuming,
            baseline,
            Duration::from_secs(2),
            Some(restored),
            now,
        ));

        // Post-resume mode-only sequence leaves last_paint_seq at baseline and tui_ready_seq > baseline.
        let mode_only = runner_terminal::terminal::TerminalOutputActivity {
            last_seq: 52,
            last_output_seq: 52,
            tui_ready_seq: 52,
            first_paint_seq: 50,
            last_paint_seq: 50,
            last_output_at: Some(now - Duration::from_secs(1)),
        };
        assert!(!evaluate_transition_settle(
            TransitionKind::Resuming,
            baseline,
            Duration::from_secs(2),
            Some(mode_only),
            now,
        ));

        // Actual post-resume visible paint advances last_paint_seq past baseline and settles.
        let repainted = runner_terminal::terminal::TerminalOutputActivity {
            last_seq: 53,
            last_output_seq: 53,
            tui_ready_seq: 52,
            first_paint_seq: 50,
            last_paint_seq: 53,
            last_output_at: Some(now),
        };
        assert!(evaluate_transition_settle(
            TransitionKind::Resuming,
            baseline,
            Duration::from_secs(2),
            Some(repainted),
            now,
        ));

        assert_eq!(
            spawned_output_seq(&serde_json::json!({ "output_seq": 7 })),
            Some(7)
        );
        assert_eq!(spawned_output_seq(&serde_json::json!({})), None);
    }
}
