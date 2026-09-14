use gpui::prelude::*;
use gpui::{div, rems, svg, AnyElement, ElementId};
use runner_backend::session::status::{
    Activity, AgentStatus, Lifecycle, ObservationSource, WaitReason,
};

use crate::{
    theme,
    ui::{button::spinner, tooltip::Tooltip},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Starting,
    Resuming,
    Working,
    Ready,
    Idle,
    Approval,
    Answer,
    NeedsYou,
    Stopped,
    Error,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusPresentation {
    pub kind: StatusKind,
    pub estimated: bool,
    pub exit_code: Option<i32>,
}

impl StatusPresentation {
    pub fn new(status: &AgentStatus) -> Self {
        let observation = &status.observation;
        let kind = match status.lifecycle {
            Lifecycle::Starting => StatusKind::Starting,
            Lifecycle::Resuming => StatusKind::Resuming,
            Lifecycle::Stopped => StatusKind::Stopped,
            Lifecycle::Error => StatusKind::Error,
            Lifecycle::Running => {
                if let Some(wait) = observation.interactions.first() {
                    match wait.reason {
                        WaitReason::Approval => StatusKind::Approval,
                        WaitReason::Answer => StatusKind::Answer,
                        WaitReason::Unknown => StatusKind::NeedsYou,
                    }
                } else {
                    match observation.activity {
                        Activity::Working => StatusKind::Working,
                        Activity::Ready => StatusKind::Ready,
                        Activity::Idle => StatusKind::Idle,
                        Activity::Unavailable => StatusKind::Unavailable,
                    }
                }
            }
        };
        Self {
            kind,
            estimated: observation.source == ObservationSource::Baseline
                && matches!(kind, StatusKind::Working | StatusKind::Idle),
            exit_code: status.exit_code,
        }
    }

    pub fn label(self) -> &'static str {
        match self.kind {
            StatusKind::Starting => "Starting",
            StatusKind::Resuming => "Resuming",
            StatusKind::Working => "Working",
            StatusKind::Ready => "Ready",
            StatusKind::Idle => "Idle",
            StatusKind::Approval => "Approval needed",
            StatusKind::Answer => "Answer needed",
            StatusKind::NeedsYou => "Needs you",
            StatusKind::Stopped => "Stopped",
            StatusKind::Error => "Error",
            StatusKind::Unavailable => "Status unavailable",
        }
    }

    pub fn needs_you(self) -> bool {
        matches!(
            self.kind,
            StatusKind::Approval | StatusKind::Answer | StatusKind::NeedsYou
        )
    }

    pub fn tooltip(self) -> String {
        if self.estimated {
            return format!("{} · estimated from terminal activity", self.label());
        }
        match self.kind {
            StatusKind::Approval => "Waiting for you to approve a command or plan".into(),
            StatusKind::Answer => "Waiting for your answer".into(),
            StatusKind::NeedsYou => "Waiting for your input in the terminal".into(),
            StatusKind::Unavailable => "Status unavailable · Agent is still connected".into(),
            StatusKind::Error => self.exit_code.map_or_else(
                || "Process exited unexpectedly".into(),
                |code| format!("Process exited · code {code}"),
            ),
            _ => self.label().into(),
        }
    }

    pub fn shows_label(self, width: f32) -> bool {
        width
            >= if self.needs_you() || self.kind == StatusKind::Error {
                320.
            } else {
                480.
            }
    }
}

pub fn status_glyph(status: StatusPresentation, id: impl Into<ElementId>) -> AnyElement {
    let color = if status.needs_you() {
        theme::warning()
    } else if status.kind == StatusKind::Error {
        theme::danger()
    } else {
        theme::muted()
    };
    let icon = match status.kind {
        StatusKind::Starting | StatusKind::Resuming | StatusKind::Working => {
            spinner(id, 12., color)
        }
        StatusKind::Ready | StatusKind::Idle => div()
            .size(rems(8. / 16.))
            .rounded_full()
            .border_1()
            .border_color(color)
            .into_any_element(),
        kind => svg()
            .path(match kind {
                StatusKind::Approval => "hand.svg",
                StatusKind::Answer => "message-circle.svg",
                StatusKind::NeedsYou => "triangle-alert.svg",
                StatusKind::Stopped => "square.svg",
                StatusKind::Error => "circle-alert.svg",
                _ => "circle-question-mark.svg",
            })
            .size(rems(12. / 16.))
            .text_color(color)
            .into_any_element(),
    };
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(rems(1. / 16.))
        .child(
            div()
                .size(rems(12. / 16.))
                .flex()
                .items_center()
                .justify_center()
                .child(icon),
        )
        .when(status.estimated, |row| {
            row.child(
                div()
                    .text_size(rems(9. / 16.))
                    .text_color(theme::muted())
                    .child("~"),
            )
        })
        .into_any_element()
}

pub fn status_indicator(
    status: StatusPresentation,
    label: bool,
    id: impl Into<ElementId>,
) -> AnyElement {
    let id = id.into();
    let color = if status.needs_you() {
        theme::warning()
    } else if status.kind == StatusKind::Error {
        theme::danger()
    } else {
        theme::muted()
    };
    let row = div()
        .flex_none()
        .flex()
        .items_center()
        .gap(rems(5. / 16.))
        .child(status_glyph(status, (id.clone(), "glyph")))
        .when(label, |row| {
            row.child(
                div()
                    .text_size(rems(11. / 16.))
                    .text_color(color)
                    .whitespace_nowrap()
                    .child(status.label()),
            )
        });
    Tooltip::new((id, "tooltip"), status.tooltip(), row).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use runner_backend::session::status::AgentObservation;

    #[test]
    fn baseline_idle_never_claims_ready_and_narrow_labels_preserve_attention() {
        let status = AgentStatus {
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
        assert!(presentation.estimated);
        assert!(!presentation.shows_label(360.));
        let approval = StatusPresentation {
            kind: StatusKind::Approval,
            estimated: false,
            exit_code: None,
        };
        assert!(approval.shows_label(360.));
        assert!(!approval.shows_label(296.));
    }
}

#[derive(Clone, Debug, Default)]
pub struct StatusRollup {
    pub entries: Vec<(String, AgentStatus)>,
}

impl StatusRollup {
    pub fn priority(status: &AgentStatus) -> u8 {
        if status.error_since.is_some() {
            5
        } else if status.lifecycle == Lifecycle::Running && status.observation.needs_you() {
            4
        } else if status.lifecycle == Lifecycle::Running
            && status.observation.activity == Activity::Working
        {
            3
        } else if status.unread_since.is_some() {
            2
        } else if status.lifecycle == Lifecycle::Running
            && status.observation.activity == Activity::Unavailable
        {
            1
        } else {
            0
        }
    }

    pub fn target(&self) -> Option<&str> {
        self.dominant().map(|(id, _)| id.as_str())
    }

    fn dominant(&self) -> Option<&(String, AgentStatus)> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, (_, status))| Self::priority(status) > 0)
            .min_by_key(|(index, (_, status))| {
                (
                    std::cmp::Reverse(Self::priority(status)),
                    status
                        .error_since
                        .or_else(|| {
                            status
                                .observation
                                .interactions
                                .first()
                                .map(|wait| wait.since)
                        })
                        .unwrap_or(i64::MAX),
                    *index,
                )
            })
            .map(|(_, entry)| entry)
    }

    pub fn priority_value(&self) -> u8 {
        self.dominant()
            .map_or(0, |(_, status)| Self::priority(status))
    }

    pub fn tooltip(&self) -> String {
        let mut counts = [0; 7];
        let mut estimated = 0;
        for (_, status) in &self.entries {
            counts[0] += usize::from(status.error_since.is_some());
            if status.lifecycle == Lifecycle::Running {
                for wait in &status.observation.interactions {
                    counts[match wait.reason {
                        WaitReason::Approval => 1,
                        WaitReason::Answer => 2,
                        WaitReason::Unknown => 3,
                    }] += 1;
                }
                counts[4] += usize::from(
                    status.observation.activity == Activity::Working
                        && !status.observation.needs_you(),
                );
                estimated += usize::from(
                    status.observation.activity == Activity::Working
                        && status.observation.source == ObservationSource::Baseline,
                );
                counts[6] += usize::from(
                    status.observation.activity == Activity::Unavailable
                        && !status.observation.needs_you(),
                );
            }
            counts[5] += usize::from(status.unread_since.is_some());
        }
        let counts = counts
            .iter()
            .zip([
                "error",
                "approval needed",
                "answer needed",
                "needs you",
                "working",
                "unread response",
                "status unavailable",
            ])
            .filter(|(count, _)| **count > 0)
            .map(|(count, label)| format!("{count} {label}"))
            .collect::<Vec<_>>()
            .join(" · ");
        if estimated > 0 {
            format!("{counts} · {estimated} estimated from terminal activity")
        } else {
            counts
        }
    }

    pub fn render(&self, id: impl Into<ElementId>) -> AnyElement {
        let Some((_, status)) = self.dominant() else {
            return div().into_any_element();
        };
        let id = id.into();
        let glyph = if Self::priority(status) == 2 {
            div()
                .size(rems(6. / 16.))
                .rounded_full()
                .bg(theme::accent())
                .into_any_element()
        } else {
            status_glyph(StatusPresentation::new(status), (id.clone(), "glyph"))
        };
        Tooltip::new(
            (id, "tooltip"),
            self.tooltip(),
            div()
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .min_w(rems(12. / 16.))
                .child(glyph),
        )
        .into_any_element()
    }
}

#[cfg(test)]
mod rollup_tests {
    use super::*;
    use runner_backend::session::status::{AgentObservation, HumanInteraction};

    fn working() -> AgentStatus {
        AgentStatus {
            lifecycle: Lifecycle::Running,
            observation: AgentObservation {
                activity: Activity::Working,
                source: ObservationSource::Hook,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn waiting(since: i64) -> AgentStatus {
        let mut status = working();
        status.observation.interactions.push(HumanInteraction {
            id: format!("wait-{since}"),
            reason: WaitReason::Approval,
            owners: vec![],
            since,
        });
        status
    }

    #[test]
    fn oldest_error_then_oldest_request_is_stable_and_counts_survive_priority() {
        let mut error = working();
        error.lifecycle = Lifecycle::Error;
        error.error_since = Some(30);
        let mut rollup = StatusRollup {
            entries: vec![
                ("newer".into(), waiting(20)),
                ("oldest".into(), waiting(10)),
                ("peer".into(), working()),
                ("error".into(), error),
            ],
        };
        assert_eq!(rollup.target(), Some("error"));
        assert_eq!(rollup.tooltip(), "1 error · 2 approval needed · 1 working");
        rollup.entries[3].1.error_since = None;
        for _ in 0..3 {
            assert_eq!(rollup.target(), Some("oldest"));
        }
        assert_eq!(rollup.entries[3].1.lifecycle, Lifecycle::Error);
        rollup.entries[0].1.observation.interactions[0].since = 10;
        assert_eq!(rollup.target(), Some("newer"));
    }

    #[test]
    fn new_work_masks_unread_without_deleting_it_and_estimated_idle_is_quiet() {
        let mut status = working();
        status.unread_since = Some(1);
        let mut rollup = StatusRollup {
            entries: vec![("response".into(), status)],
        };
        assert_eq!(rollup.priority_value(), 3);
        assert_eq!(rollup.tooltip(), "1 working · 1 unread response");
        rollup.entries[0].1.observation.activity = Activity::Ready;
        assert_eq!(rollup.priority_value(), 2);
        rollup.entries[0].1.unread_since = None;
        assert_eq!(rollup.priority_value(), 0);
        rollup.entries[0].1.observation.activity = Activity::Idle;
        rollup.entries[0].1.observation.source = ObservationSource::Baseline;
        assert_eq!(rollup.priority_value(), 0);
        rollup.entries[0].1.observation.activity = Activity::Unavailable;
        assert_eq!(rollup.priority_value(), 1);
    }
}
