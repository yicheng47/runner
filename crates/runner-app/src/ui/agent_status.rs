use std::time::Duration;

use chrono::Utc;
use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, App, ElementId, FontWeight, SharedString, Task, Window,
};
use runner_backend::session::status::{
    Activity, AgentStatus, Lifecycle, ObservationSource, TurnOutcome, WaitReason, WorkDetail,
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
    Idle,
    Approval,
    Answer,
    NeedsYou,
    Stopped,
    Error,
    ResponseFailed,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusPresentation {
    pub kind: StatusKind,
    pub estimated: bool,
    pub exit_code: Option<i32>,
    pub outcome: Option<TurnOutcome>,
    pub waiting_since: Option<i64>,
    pub detail: Option<WorkDetail>,
}

struct ShortStatusLabel {
    base: &'static str,
    detail: Option<String>,
}

impl ShortStatusLabel {
    #[cfg(test)]
    fn text(&self) -> String {
        self.detail.as_ref().map_or_else(
            || self.base.into(),
            |detail| format!("{} · {detail}", self.base),
        )
    }
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
                } else if observation.outcome == Some(TurnOutcome::Failed) {
                    StatusKind::ResponseFailed
                } else {
                    match observation.activity {
                        Activity::Working => StatusKind::Working,
                        Activity::Ready | Activity::Idle => StatusKind::Idle,
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
            outcome: observation.outcome,
            waiting_since: observation.interactions.first().map(|wait| wait.since),
            detail: observation.detail,
        }
    }

    pub fn label(self) -> &'static str {
        match self.kind {
            StatusKind::Starting => "Starting",
            StatusKind::Resuming => "Resuming",
            StatusKind::Working => "Working",
            StatusKind::Idle => "Idle",
            StatusKind::Approval => "Approval needed",
            StatusKind::Answer => "Answer needed",
            StatusKind::NeedsYou => "Needs you",
            StatusKind::Stopped => "Stopped",
            StatusKind::Error => "Error",
            StatusKind::ResponseFailed => "Response failed",
            StatusKind::Unavailable => "Status unavailable",
        }
    }

    pub fn needs_you(self) -> bool {
        matches!(
            self.kind,
            StatusKind::Approval | StatusKind::Answer | StatusKind::NeedsYou
        )
    }

    pub fn is_error(self) -> bool {
        matches!(self.kind, StatusKind::Error | StatusKind::ResponseFailed)
    }

    pub fn tooltip(self) -> String {
        self.tooltip_at(Utc::now().timestamp_millis())
    }

    fn tooltip_at(self, now: i64) -> String {
        if self.estimated {
            return format!("{} · estimated from terminal activity", self.label());
        }
        let tooltip = match self.kind {
            StatusKind::Approval => "Waiting for you to approve a command or plan".into(),
            StatusKind::Answer => "Waiting for your answer".into(),
            StatusKind::NeedsYou => "Waiting for your input in the terminal".into(),
            StatusKind::Unavailable => "Status unavailable · Agent is still connected".into(),
            StatusKind::ResponseFailed => "Response failed · Agent is still connected".into(),
            StatusKind::Error => self.exit_code.map_or_else(
                || "Process exited unexpectedly".into(),
                |code| format!("Process exited · code {code}"),
            ),
            StatusKind::Idle if self.outcome == Some(TurnOutcome::Interrupted) => {
                "Idle · Last response interrupted".into()
            }
            StatusKind::Working => self.detail.map_or_else(
                || "Working".into(),
                |detail| format!("Working · {}", work_detail_label(detail)),
            ),
            _ => self.label().into(),
        };
        if matches!(
            self.kind,
            StatusKind::Approval | StatusKind::Answer | StatusKind::NeedsYou
        ) {
            if let Some(since) = self.waiting_since {
                return format!("{tooltip} · {}", format_elapsed(now.saturating_sub(since)));
            }
        }
        tooltip
    }

    fn short_label_at(self, now: i64) -> ShortStatusLabel {
        let detail = if self.estimated {
            None
        } else if self.kind == StatusKind::Working {
            self.detail.map(|detail| work_detail_label(detail).into())
        } else if self.kind == StatusKind::Idle && self.outcome == Some(TurnOutcome::Interrupted) {
            Some("Interrupted".into())
        } else if matches!(
            self.kind,
            StatusKind::Approval | StatusKind::Answer | StatusKind::NeedsYou
        ) {
            self.waiting_since.and_then(|since| {
                let elapsed = now.saturating_sub(since);
                (elapsed >= 60_000).then(|| format_elapsed(elapsed))
            })
        } else {
            None
        };
        ShortStatusLabel {
            base: self.label(),
            detail,
        }
    }

    pub fn shows_detail(self, width: f32) -> bool {
        let can_have_detail = self.kind == StatusKind::Working
            || self.needs_you()
            || (self.kind == StatusKind::Idle && self.outcome == Some(TurnOutcome::Interrupted));
        can_have_detail && width >= if self.needs_you() { 400. } else { 560. }
    }

    #[cfg(test)]
    fn visible_short_label_at(self, width: f32, now: i64) -> Option<String> {
        if !self.shows_label(width) {
            return None;
        }
        let mut label = self.short_label_at(now);
        if !self.shows_detail(width) {
            label.detail = None;
        }
        Some(label.text())
    }

    pub fn shows_label(self, width: f32) -> bool {
        width
            >= if self.needs_you() || self.is_error() {
                320.
            } else {
                480.
            }
    }
}

fn work_detail_label(detail: WorkDetail) -> &'static str {
    match detail {
        WorkDetail::UsingTools => "Using tools",
        WorkDetail::CompactingContext => "Compacting context",
    }
}

pub fn format_elapsed(ms: i64) -> String {
    let seconds = ms.max(0) / 1_000;
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m", seconds / 60)
    } else {
        format!("{}h {}m", seconds / 3_600, seconds % 3_600 / 60)
    }
}

pub fn status_glyph(status: StatusPresentation, id: impl Into<ElementId>) -> AnyElement {
    let color = if status.needs_you() {
        theme::warning()
    } else if status.is_error() {
        theme::danger()
    } else {
        theme::muted()
    };
    let icon = match status.kind {
        StatusKind::Starting | StatusKind::Resuming | StatusKind::Working => {
            spinner(id, 12., color)
        }
        StatusKind::Idle => div()
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
                StatusKind::Error | StatusKind::ResponseFailed => "circle-alert.svg",
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
        .into_any_element()
}

pub fn status_indicator(
    status: StatusPresentation,
    label: bool,
    id: impl Into<ElementId>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    timed_status_indicator(status, label, true, id.into(), false, window, cx)
}

fn timed_status_indicator(
    status: StatusPresentation,
    label: bool,
    detail: bool,
    id: ElementId,
    header: bool,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    if let Some(since) = status.waiting_since.filter(|_| status.needs_you()) {
        let refresh_id = (
            id.clone(),
            SharedString::from(format!("minute-refresh-{since}")),
        );
        let _ticker = window.use_keyed_state(refresh_id, cx, |_window, cx| {
            let refresh_task = cx.spawn(async move |weak, cx| loop {
                let elapsed = Utc::now().timestamp_millis().saturating_sub(since).max(0);
                let delay = 60_000 - elapsed.rem_euclid(60_000);
                cx.background_executor()
                    .timer(Duration::from_millis(delay as u64))
                    .await;
                if weak.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            });
            MinuteTicker {
                _refresh_task: refresh_task,
            }
        });
    }
    render_status_indicator(
        status,
        label,
        detail,
        id,
        header,
        Utc::now().timestamp_millis(),
    )
}

struct MinuteTicker {
    _refresh_task: Task<()>,
}

pub fn pane_status_indicator(
    status: StatusPresentation,
    label: bool,
    detail: bool,
    id: impl Into<ElementId>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    timed_status_indicator(status, label, detail, id.into(), true, window, cx)
}

pub fn header_status_indicator(
    status: StatusPresentation,
    label: bool,
    detail: bool,
    id: impl Into<ElementId>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let width = header_status_width(label, detail);
    let indicator = pane_status_indicator(status, label, detail, id, window, cx);
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(rems(10. / 16.))
        .child(
            div()
                .flex_none()
                .w(rems(1. / 16.))
                .h(rems(14. / 16.))
                .bg(theme::border_strong()),
        )
        .child(
            div()
                .flex_none()
                .when(label && !detail, |slot| slot.min_w(rems(width / 16.)))
                .when(!label || detail, |slot| slot.w(rems(width / 16.)))
                .child(indicator),
        )
        .into_any_element()
}

fn header_status_width(label: bool, detail: bool) -> f32 {
    if !label {
        16.
    } else if detail {
        180.
    } else {
        108.
    }
}

fn render_status_indicator(
    status: StatusPresentation,
    label: bool,
    detail: bool,
    id: ElementId,
    header: bool,
    now: i64,
) -> AnyElement {
    let color = if status.needs_you() {
        theme::warning()
    } else if status.is_error() {
        theme::danger()
    } else {
        theme::muted()
    };
    let short_label = status.short_label_at(now);
    let text_size = rems(
        if header && !status.needs_you() && !status.is_error() {
            10.
        } else {
            11.
        } / 16.,
    );
    let row = div()
        .flex_none()
        .flex()
        .when(!header, |row| row.w_full().min_w(px(0.)))
        .when(header && label && !detail, |row| {
            row.min_w(rems(header_status_width(label, detail) / 16.))
        })
        .when(header && (!label || detail), |row| {
            row.w(rems(header_status_width(label, detail) / 16.))
        })
        .items_center()
        .gap(rems(5. / 16.))
        .when(
            !header || !label || !matches!(status.kind, StatusKind::Stopped | StatusKind::Idle),
            |row| row.child(status_glyph(status, (id.clone(), "glyph"))),
        )
        .when(label, |row| {
            row.child(
                div()
                    .flex_none()
                    .text_size(text_size)
                    .text_color(color)
                    .whitespace_nowrap()
                    .child(short_label.base),
            )
        })
        .children(
            (label && (!header || detail))
                .then_some(short_label.detail)
                .flatten()
                .map(|detail| {
                    div()
                        .min_w(px(0.))
                        .flex_1()
                        .truncate()
                        .font_weight(FontWeight::NORMAL)
                        .text_size(text_size)
                        .text_color(theme::muted())
                        .whitespace_nowrap()
                        .child(format!("· {detail}"))
                }),
        );
    status_tooltip(status, (id, "tooltip"), row)
}

fn status_tooltip(
    status: StatusPresentation,
    id: impl Into<ElementId>,
    child: impl IntoElement,
) -> AnyElement {
    status_tooltip_with_unread(status, false, id, child)
}

fn status_tooltip_with_unread(
    status: StatusPresentation,
    unread: bool,
    id: impl Into<ElementId>,
    child: impl IntoElement,
) -> AnyElement {
    if status.needs_you() && status.waiting_since.is_some() {
        Tooltip::refreshing(
            id,
            move || {
                SharedString::from(status_tooltip_text_at(
                    status,
                    unread,
                    Utc::now().timestamp_millis(),
                ))
            },
            Duration::from_secs(1),
            child,
        )
        .into_any_element()
    } else {
        Tooltip::new(
            id,
            status_tooltip_text_at(status, unread, Utc::now().timestamp_millis()),
            child,
        )
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runner_backend::session::status::{AgentObservation, HumanInteraction};

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
                ..plain
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
        if status.error_since.is_some() || status.failed_since.is_some() {
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
                        .into_iter()
                        .chain(status.failed_since)
                        .min()
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
        let mut counts = [0; 8];
        let mut estimated = 0;
        for (_, status) in &self.entries {
            counts[0] += usize::from(status.failed_since.is_some());
            counts[1] += usize::from(status.error_since.is_some());
            if status.lifecycle == Lifecycle::Running {
                for wait in &status.observation.interactions {
                    counts[match wait.reason {
                        WaitReason::Approval => 2,
                        WaitReason::Answer => 3,
                        WaitReason::Unknown => 4,
                    }] += 1;
                }
                counts[5] += usize::from(
                    status.observation.activity == Activity::Working
                        && !status.observation.needs_you(),
                );
                estimated += usize::from(
                    status.observation.activity == Activity::Working
                        && status.observation.source == ObservationSource::Baseline,
                );
                counts[7] += usize::from(
                    status.observation.activity == Activity::Unavailable
                        && !status.observation.needs_you(),
                );
            }
            counts[6] += usize::from(status.unread_since.is_some());
        }
        let counts = counts
            .iter()
            .zip([
                "response failed",
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
        self.render_with_pane_tooltip(id.into(), false)
    }

    pub fn render_pane(&self, id: impl Into<ElementId>) -> AnyElement {
        self.render_with_pane_tooltip(id.into(), true)
    }

    fn render_with_pane_tooltip(&self, id: ElementId, pane_tooltip: bool) -> AnyElement {
        let Some((_, status)) = self.dominant() else {
            return div().into_any_element();
        };
        let glyph = if Self::priority(status) == 2 {
            div()
                .size(rems(6. / 16.))
                .rounded_full()
                .bg(theme::accent())
                .into_any_element()
        } else {
            status_glyph(attention_presentation(status), (id.clone(), "glyph"))
        };
        let indicator = div()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .min_w(rems(12. / 16.))
            .child(glyph);
        if pane_tooltip && pane_uses_status_tooltip(status) {
            status_tooltip_with_unread(
                attention_presentation(status),
                status.unread_since.is_some(),
                (id, "tooltip"),
                indicator,
            )
        } else {
            Tooltip::new((id, "tooltip"), self.tooltip(), indicator).into_any_element()
        }
    }
}

fn attention_presentation(status: &AgentStatus) -> StatusPresentation {
    let mut presentation = StatusPresentation::new(status);
    if status.failed_since.is_some() {
        presentation.kind = StatusKind::ResponseFailed;
    }
    presentation
}

fn pane_uses_status_tooltip(status: &AgentStatus) -> bool {
    StatusRollup::priority(status) != 2
}

fn status_tooltip_text_at(status: StatusPresentation, unread: bool, now: i64) -> String {
    let tooltip = status.tooltip_at(now);
    if unread {
        format!("{tooltip} · 1 unread response")
    } else {
        tooltip
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
    fn response_failure_ranks_with_error_and_targets_the_oldest_attention() {
        let mut failed = working();
        failed.observation.activity = Activity::Ready;
        failed.observation.outcome = Some(TurnOutcome::Failed);
        failed.failed_since = Some(5);
        let mut error = working();
        error.lifecycle = Lifecycle::Error;
        error.error_since = Some(10);
        let rollup = &mut StatusRollup {
            entries: vec![
                ("wait".into(), waiting(1)),
                ("working".into(), working()),
                ("failed".into(), failed),
                ("error".into(), error),
            ],
        };
        assert_eq!(StatusRollup::priority(&rollup.entries[2].1), 5);
        assert_eq!(rollup.priority_value(), 5);
        assert_eq!(rollup.target(), Some("failed"));
        assert_eq!(
            rollup.tooltip(),
            "1 response failed · 1 error · 1 approval needed · 1 working"
        );

        rollup.entries[2].1.failed_since = Some(20);
        assert_eq!(rollup.target(), Some("error"));
        rollup.entries[2].1.failed_since = Some(10);
        assert_eq!(rollup.target(), Some("failed"));
    }

    #[test]
    fn interrupted_outcome_adds_no_rollup_attention() {
        let mut interrupted = working();
        interrupted.observation.activity = Activity::Ready;
        interrupted.observation.outcome = Some(TurnOutcome::Interrupted);
        let rollup = StatusRollup {
            entries: vec![("interrupted".into(), interrupted)],
        };
        assert_eq!(rollup.priority_value(), 0);
        assert_eq!(rollup.target(), None);
        assert_eq!(rollup.tooltip(), "");
    }

    #[test]
    fn single_pane_unread_dot_keeps_its_rollup_tooltip() {
        let mut unread = working();
        unread.observation.activity = Activity::Ready;
        unread.unread_since = Some(1);
        assert_eq!(StatusRollup::priority(&unread), 2);
        assert!(!pane_uses_status_tooltip(&unread));
        assert_eq!(
            StatusRollup {
                entries: vec![("unread".into(), unread.clone())],
            }
            .tooltip(),
            "1 unread response"
        );

        unread.observation.activity = Activity::Working;
        unread.observation.detail = Some(WorkDetail::UsingTools);
        assert_eq!(StatusRollup::priority(&unread), 3);
        assert!(pane_uses_status_tooltip(&unread));
        assert_eq!(
            status_tooltip_text_at(StatusPresentation::new(&unread), true, 0),
            "Working · Using tools · 1 unread response"
        );

        unread.observation.interactions.push(HumanInteraction {
            id: "approval".into(),
            reason: WaitReason::Approval,
            owners: vec![],
            since: 0,
        });
        assert_eq!(
            status_tooltip_text_at(StatusPresentation::new(&unread), true, 120_000),
            "Waiting for you to approve a command or plan · 2m · 1 unread response"
        );
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
