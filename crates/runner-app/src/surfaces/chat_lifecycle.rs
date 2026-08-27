use std::path::Path;
use std::time::Duration;

use runner_backend::model::SessionStatus;

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

pub(crate) fn ended_subtitle(
    status: SessionStatus,
    resumable: bool,
    exit_code: Option<i32>,
) -> String {
    if !resumable {
        return "The PTY is closed. Resume to start a fresh agent process — there's no saved conversation to pick up from this row.".into();
    }
    if status == SessionStatus::Crashed {
        return exit_code.map_or_else(
            || "The PTY exited unexpectedly. Resume to start a fresh process — the prior agent conversation is preserved.".into(),
            |code| format!("The PTY exited with code {code}. Resume to start a fresh process — the prior agent conversation is preserved."),
        );
    }
    "The PTY is closed, but the conversation is preserved. Resume to pick up where you left off."
        .into()
}

pub(crate) fn shell_exited_subtitle(
    exit_code: Option<i32>,
    shell: &str,
    cwd: Option<&str>,
    home: Option<&str>,
) -> String {
    let exit = exit_code.map_or_else(|| "exit unknown".into(), |code| format!("exit {code}"));
    let cwd = cwd.map_or_else(
        || "unknown cwd".to_owned(),
        |cwd| {
            home.and_then(|home| Path::new(cwd).strip_prefix(home).ok())
                .map(|relative| {
                    if relative.as_os_str().is_empty() {
                        "~".to_owned()
                    } else {
                        format!("~/{}", relative.display())
                    }
                })
                .unwrap_or_else(|| cwd.to_owned())
        },
    );
    format!("{exit} · {shell} · {cwd}")
}

/// The pill drops on the first painted frame. The output-idle backstop covers
/// processes that never paint, but not one that has taken the screen
/// (`tui_ready_seen`) and is still booting behind it: claude-code's fullscreen
/// renderer enters the alternate screen at process start and paints its first
/// frame 1–2 s later, and dropping the pill on the silence between shows a
/// blank pane.
pub(crate) fn transition_should_settle(
    kind: TransitionKind,
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
        || (kind == TransitionKind::Starting && !output_seen && elapsed >= TRANSITION_MIN_VISIBLE)
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
    fn shell_exit_copy_includes_code_shell_and_abbreviated_cwd() {
        assert_eq!(
            shell_exited_subtitle(
                Some(0),
                "zsh",
                Some("/Users/jason/workspace/oec"),
                Some("/Users/jason")
            ),
            "exit 0 · zsh · ~/workspace/oec"
        );
        assert_eq!(
            shell_exited_subtitle(None, "fish", Some("/tmp/project"), Some("/Users/jason")),
            "exit unknown · fish · /tmp/project"
        );
    }

    #[test]
    fn transition_settling_uses_first_paint_with_timeout_and_idle_backstops() {
        assert!(transition_should_settle(
            TransitionKind::Resuming,
            Duration::from_millis(20),
            true,
            true,
            true,
            Some(Duration::ZERO),
        ));
        assert!(!transition_should_settle(
            TransitionKind::Resuming,
            Duration::from_secs(1),
            false,
            false,
            false,
            None,
        ));
        assert!(transition_should_settle(
            TransitionKind::Starting,
            Duration::from_secs(1),
            false,
            false,
            false,
            None,
        ));
        assert!(transition_should_settle(
            TransitionKind::Resuming,
            Duration::from_secs(1),
            false,
            false,
            true,
            Some(Duration::from_millis(400)),
        ));
        assert!(transition_should_settle(
            TransitionKind::Resuming,
            Duration::from_secs(10),
            false,
            false,
            false,
            None,
        ));
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
}
