use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, Context, FontWeight, IntoElement, MouseButton, SharedString, Window,
};
use runner_core::protocol::mission_control::{MissionControlLane, MissionControlSnapshot};
use std::path::PathBuf;

use crate::{file_links::FileLinkTarget, theme, NativeRoot};

#[derive(Default)]
pub(crate) struct MissionControlState {
    project_id: Option<String>,
    snapshot: Option<MissionControlSnapshot>,
    error: Option<String>,
    generation: u64,
    loading: bool,
}

impl NativeRoot {
    pub(crate) fn render_mission_control_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let project_id = self.active_project_id(cx);
        let projects = self.app_store.read(cx).projects.clone();
        let Some(project) = projects
            .iter()
            .find(|project| Some(project.id.as_str()) == project_id.as_deref())
        else {
            return empty_state("Open a project to read its local tickets and triage evidence.");
        };
        if self.mission_control.project_id.as_deref() != Some(project.id.as_str()) {
            self.refresh_mission_control(project.id.clone(), cx);
        }
        match (&self.mission_control.snapshot, &self.mission_control.error) {
            (Some(snapshot), _) => {
                self.render_mission_control_snapshot(snapshot.clone(), window, cx)
            }
            (_, Some(error)) => empty_state(format!("Could not read local evidence: {error}")),
            _ => empty_state("Loading local tickets and triage evidence…"),
        }
    }

    /// Loads the read-only daemon snapshot outside GPUI's render pass.
    pub(crate) fn refresh_mission_control(&mut self, project_id: String, cx: &mut Context<Self>) {
        if self.remote_client.is_active()
            || self.mission_control.loading
                && self.mission_control.project_id.as_deref() == Some(project_id.as_str())
        {
            return;
        }
        self.mission_control.project_id = Some(project_id.clone());
        self.mission_control.snapshot = None;
        self.mission_control.error = None;
        self.mission_control.loading = true;
        self.mission_control.generation += 1;
        let generation = self.mission_control.generation;
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move { core.mission_control_snapshot(&project_id) });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                if this.mission_control.generation != generation {
                    return;
                }
                this.mission_control.loading = false;
                match result {
                    Ok(snapshot) => this.mission_control.snapshot = Some(snapshot),
                    Err(error) => this.mission_control.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_mission_control_snapshot(
        &mut self,
        snapshot: MissionControlSnapshot,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let warning_count = snapshot.health.len();
        let mut lanes = mission_control_lanes()
            .into_iter()
            .map(|lane| (lane, Vec::new()))
            .collect::<Vec<_>>();
        for card in snapshot.cards {
            let (_, cards) = lanes
                .iter_mut()
                .find(|(lane, _)| *lane == card.lane)
                .expect("every Mission Control lane is rendered");
            cards.push(card);
        }
        let lanes = lanes.into_iter().map(|(lane, cards)| {
            let cards = cards.into_iter().map(|card| {
            let mission = card
                .mission
                .as_ref()
                .map(|mission| format!(" · {}", mission.title))
                .unwrap_or_default();
            let mission_id = card.mission.as_ref().map(|mission| mission.id.clone());
            let pending_question = card
                .mission
                .as_ref()
                .and_then(|mission| mission.pending_question.as_ref())
                .map(|question| format!("Needs you: {question}"));
            let handoff = card
                .mission
                .as_ref()
                .and_then(|mission| mission.newest_handoff.as_ref())
                .map(|handoff| format!("Handoff: {handoff}"));
            let details = [
                card.mission.as_ref().map(|mission| {
                    format!("{} active slot{}", mission.active_slots, if mission.active_slots == 1 { "" } else { "s" })
                }),
                card.mission
                    .as_ref()
                    .and_then(|mission| mission.branch.as_ref())
                    .map(|branch| format!("⌥ {branch}")),
                card.mission
                    .as_ref()
                    .and_then(|mission| mission.worktree.as_ref())
                    .map(|worktree| format!("⌂ {worktree}")),
            ];
            let evidence = card.evidence.to_vec();
            let card = div()
                .id(SharedString::from(format!("MISSION_CONTROL_CARD:{}", card.ticket_id)))
                .p_3()
                .rounded_md()
                .border_1()
                .border_color(theme::border())
                .bg(theme::raised())
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_xs()
                        .text_color(theme::muted())
                        .child(chip(card.ticket_id.clone(), theme::border()))
                        .children(card.severity.map(|severity| chip(severity, theme::warning())))
                        .children(card.triage_state.map(|state| chip(state, theme::accent())))
                        .child(chip(lane_label(lane).to_owned(), lane_color(lane))),
                )
                .child(div().font_weight(FontWeight::SEMIBOLD).child(card.summary))
                .child(
                    div()
                        .text_sm()
                        .text_color(theme::muted())
                        .truncate()
                        .child(mission),
                )
                .children(card.blockers.into_iter().map(|blocker| {
                    div()
                        .text_sm()
                        .text_color(theme::danger())
                        .child(format!("Blocked · {blocker}"))
                }))
                .children(pending_question.map(|question| {
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::warning())
                        .child(question)
                }))
                .children(handoff.map(|handoff| {
                    div()
                        .text_sm()
                        .text_color(theme::muted())
                        .child(handoff)
                }))
                .children(details.into_iter().flatten().map(|detail| {
                    div()
                        .text_sm()
                        .text_color(theme::muted())
                        .truncate()
                        .child(detail)
                }))
                .children(evidence.into_iter().map(|evidence| {
                    let path = evidence.path.clone();
                    let runner_mission = mission_id.clone();
                    let source_label = match evidence.kind {
                        runner_core::protocol::mission_control::MissionControlSourceKind::Ticket => "Ticket",
                        runner_core::protocol::mission_control::MissionControlSourceKind::Triage => "Triage",
                        runner_core::protocol::mission_control::MissionControlSourceKind::Runner => "Runner",
                    };
                    let evidence = div()
                        .id(SharedString::from(format!("MISSION_CONTROL_EVIDENCE:{path}")))
                        .px_1()
                        .py_1()
                        .rounded_sm()
                        .border_1()
                        .border_color(theme::border())
                        .text_xs()
                        .text_color(theme::accent())
                        .cursor_pointer()
                        .child(format!("{source_label} · {path}"))
                        .on_mouse_down(MouseButton::Left, cx.listener(move |root, _, window, cx| {
                            cx.stop_propagation();
                            if evidence.kind
                                == runner_core::protocol::mission_control::MissionControlSourceKind::Runner
                            {
                                let Some(mission_id) = runner_mission.clone() else {
                                    return;
                                };
                                root.open_mission(mission_id, window, cx);
                            } else {
                                crate::file_links::open_file_link(
                                    FileLinkTarget { path: PathBuf::from(path.clone()), line: None, column: None },
                                    cx,
                                );
                            }
                        }));
                    #[cfg(test)]
                    let evidence = crate::theme_snapshot::record_fill("MISSION_CONTROL_EVIDENCE", evidence);
                    evidence
                }));
            #[cfg(test)]
            let card = crate::theme_snapshot::record_fill("MISSION_CONTROL_CARD", card);
            if let Some(mission_id) = mission_id {
                card.cursor_pointer().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |root, _, window, cx| {
                        root.open_mission(mission_id.clone(), window, cx);
                    }),
                )
            } else {
                card
            }
            });
            let lane = div()
                .id(SharedString::from(format!("MISSION_CONTROL_LANE:{}", lane_label(lane))))
                .w(px(300.))
                .flex_none()
                .flex()
                .flex_col()
                .gap_2()
                .p_2()
                .rounded_md()
                .bg(theme::panel())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(div().size(px(8.)).rounded_full().bg(lane_color(lane)))
                                .child(lane_label(lane)),
                        )
                        .child(chip(cards.len().to_string(), theme::border())),
                )
                .children(cards);
            #[cfg(test)]
            let lane = crate::theme_snapshot::record_fill("MISSION_CONTROL_LANE", lane);
            lane
        });
        div()
            .id("mission-control")
            .size_full()
            .p(px(24.))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Mission Control"),
            )
            .child(div().text_sm().text_color(theme::muted()).child(format!(
                "Read-only evidence from {}{}",
                snapshot.project_path,
                if warning_count == 0 {
                    String::new()
                } else {
                    format!(" · {warning_count} source warning(s)")
                }
            )))
            .child(
                div()
                    .id("MISSION_CONTROL_BOARD_SCROLL")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_x_scroll()
                    .overflow_y_scroll()
                    .child(div().flex().items_start().gap_3().children(lanes)),
            )
            .into_any_element()
    }
}

fn mission_control_lanes() -> [MissionControlLane; 7] {
    [
        MissionControlLane::Closed,
        MissionControlLane::NeedsYou,
        MissionControlLane::Running,
        MissionControlLane::VerifyLand,
        MissionControlLane::Blocked,
        MissionControlLane::Ready,
        MissionControlLane::Backlog,
    ]
}

fn lane_label(lane: MissionControlLane) -> &'static str {
    match lane {
        MissionControlLane::Closed => "Closed",
        MissionControlLane::NeedsYou => "Needs you",
        MissionControlLane::Running => "Running",
        MissionControlLane::VerifyLand => "Verify / Land",
        MissionControlLane::Blocked => "Blocked",
        MissionControlLane::Ready => "Ready",
        MissionControlLane::Backlog => "Backlog",
    }
}

fn lane_color(lane: MissionControlLane) -> gpui::Hsla {
    match lane {
        MissionControlLane::NeedsYou | MissionControlLane::VerifyLand => theme::warning(),
        MissionControlLane::Running | MissionControlLane::Ready => theme::accent(),
        MissionControlLane::Blocked => theme::danger(),
        MissionControlLane::Closed | MissionControlLane::Backlog => theme::muted(),
    }
}

fn chip(label: impl Into<SharedString>, color: gpui::Hsla) -> AnyElement {
    div()
        .px_1()
        .py_1()
        .rounded_sm()
        .border_1()
        .border_color(color)
        .text_xs()
        .text_color(color)
        .child(label.into())
        .into_any_element()
}

fn empty_state(description: impl Into<String>) -> AnyElement {
    let empty = div().id("MISSION_CONTROL_EMPTY");
    #[cfg(test)]
    let empty = crate::theme_snapshot::record_fill("MISSION_CONTROL_EMPTY", empty);
    empty
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Mission Control"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(theme::muted())
                        .child(description.into()),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px, size, Modifiers, TestAppContext, VisualTestContext};
    use runner_core::protocol::mission_control::{
        MissionControlCard, MissionControlEvidence, MissionControlLane, MissionControlMission,
        MissionControlSourceKind,
    };

    #[test]
    fn lanes_follow_the_derived_precedence_order() {
        assert_eq!(
            mission_control_lanes(),
            [
                MissionControlLane::Closed,
                MissionControlLane::NeedsYou,
                MissionControlLane::Running,
                MissionControlLane::VerifyLand,
                MissionControlLane::Blocked,
                MissionControlLane::Ready,
                MissionControlLane::Backlog,
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn mission_control_route_renders_and_is_persisted() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let host = cx.add_window(|window, cx| {
            NativeRoot::new(
                "mission-control".into(),
                temp.path().join("logs"),
                Some("/mission-control".into()),
                None,
                store.clone(),
                window,
                cx,
            )
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(1200.), px(800.)));
        visual.run_until_parked();

        assert!(visual.debug_bounds("MISSION_CONTROL_EMPTY").is_some());
        host.read_with(&cx, |root, _| {
            assert_eq!(root.route, crate::AppRoute::MissionControl);
            assert_eq!(
                root.persisted_window_route().as_deref(),
                Some("/mission-control")
            );
            assert_eq!(
                root.runtime_navigation_history,
                vec![crate::RuntimeLocation::Entity(
                    crate::AppRoute::MissionControl
                )]
            );
        })
        .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn populated_card_and_runner_evidence_open_the_linked_mission() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let (project, mission, tab_id) = store.read_with(&cx, |store, _| {
            let mut conn = store.test_core.db.get().unwrap();
            let project = runner_daemon::ops::project::project_create(
                &store.test_core,
                "Board".into(),
                temp.path().display().to_string(),
            )
            .unwrap();
            let project_node = runner_daemon::repo::node::find_by_ref(
                &conn,
                runner_core::protocol::node::NodeType::Project,
                &project.id,
            )
            .unwrap()
            .unwrap();
            let layout = runner_app::pane_layout::PaneLayout::single(None, &[]);
            let tab_id = runner_daemon::repo::node::create_tab(
                &conn,
                Some(&project_node.id),
                "Board tab",
                0,
                &layout.serialize().unwrap(),
            )
            .unwrap()
            .id;
            let mission = runner_daemon::ops::mission::start(
                &mut conn,
                temp.path(),
                runner_core::protocol::mission::StartMissionInput {
                    project_id: Some(project.id.clone()),
                    crew_id: None,
                    title: "CR-1 review".into(),
                    goal_override: None,
                    cwd: Some(temp.path().display().to_string()),
                    started_by_session_id: None,
                    no_report: false,
                },
            )
            .unwrap()
            .mission;
            (project, mission, tab_id)
        });
        store.update(&mut cx, |store, cx| {
            store.refresh(crate::app_store::StoreRefreshKind::All, cx)
        });
        cx.run_until_parked();
        let snapshot = MissionControlSnapshot {
            project_id: project.id.clone(),
            project_path: temp.path().display().to_string(),
            cards: vec![MissionControlCard {
                ticket_id: "CR-1".into(),
                summary: "Review the populated board".into(),
                severity: Some("high".into()),
                blockers: vec!["needs evidence".into()],
                triage_state: Some("review".into()),
                lane: MissionControlLane::Closed,
                mission: Some(MissionControlMission {
                    id: mission.id.clone(),
                    title: mission.title.clone(),
                    active_slots: 1,
                    pending_question: Some("Approve?".into()),
                    newest_handoff: Some("handoff ready".into()),
                    branch: Some("feature/board".into()),
                    worktree: Some(temp.path().display().to_string()),
                }),
                evidence: vec![MissionControlEvidence {
                    kind: MissionControlSourceKind::Runner,
                    path: format!("mission:{}", mission.id),
                }],
                provenance: vec![],
            }],
            health: vec![],
        };
        let host = cx.add_window(|window, cx| {
            let mut root = NativeRoot::new(
                "mission-control-populated".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.tabs.activate(&tab_id);
            root.sync_active_project_from_active_tab(cx);
            root.mission_control.project_id = Some(project.id.clone());
            root.mission_control.snapshot = Some(snapshot.clone());
            root.set_route(crate::AppRoute::MissionControl, cx);
            root
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(1200.), px(800.)));
        visual.run_until_parked();

        let card = visual.debug_bounds("MISSION_CONTROL_CARD").unwrap();
        assert!(visual.debug_bounds("MISSION_CONTROL_LANE").is_some());
        let evidence = visual
            .debug_bounds("MISSION_CONTROL_EVIDENCE")
            .expect("Runner evidence renders on populated cards");
        assert!(evidence.top() > card.top());
        visual.simulate_click(evidence.center(), Modifiers::default());
        visual.run_until_parked();
        host.read_with(&cx, |root, _| {
            assert_eq!(root.route, crate::AppRoute::Mission(mission.id.clone()))
        })
        .unwrap();

        host.update(&mut cx, |root, _, cx| {
            root.mission_control.snapshot = Some(snapshot);
            root.set_route(crate::AppRoute::MissionControl, cx);
        })
        .unwrap();
        visual.run_until_parked();
        let card = visual.debug_bounds("MISSION_CONTROL_CARD").unwrap();
        visual.simulate_click(
            point(card.left() + px(8.), card.top() + px(8.)),
            Modifiers::default(),
        );
        visual.run_until_parked();
        host.read_with(&cx, |root, _| {
            assert_eq!(root.route, crate::AppRoute::Mission(mission.id.clone()))
        })
        .unwrap();
    }
}
