//! A chat and its linked missions as one sidebar group (#748): which rows
//! group together, the collapsed summary, the drop zones on a chat row and
//! the Link to chat picker's list.

use std::collections::HashMap;

use gpui::{prelude::*, relative, rems, AnyElement, Context, SharedString};
use runner_core::protocol::model::Runtime;
use runner_core::protocol::MissionPhase;

use super::{MissionSummary, SessionDisplay, Sidebar, SidebarRow};
use crate::surfaces::sidebar::logic::{AttentionState, DropKind};
use crate::theme;

/// The label a reorder line carries when its drop would change a mission's
/// link: the chat it would link to, or the one it would leave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DropHint {
    Link(String),
    Unlink(String),
}

/// The missions each tab row heads, by node id, in their node order.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct LinkGroups {
    by_tab: HashMap<String, Vec<String>>,
    head_of: HashMap<String, String>,
}

impl LinkGroups {
    /// A mission joins the group of the tab that holds its linked chat. A mission
    /// whose chat is archived, or in no tab, stays a flat row.
    pub(super) fn of(rows: &[SidebarRow]) -> Self {
        let tab_of_session = rows
            .iter()
            .filter_map(|row| match row {
                SidebarRow::Tab { node, members, .. } => Some((node, members)),
                SidebarRow::Mission { .. } => None,
            })
            .flat_map(|(node, members)| {
                members
                    .iter()
                    .filter(|member| is_agent_chat(member))
                    .map(move |member| (member.session_id.as_str(), node.id.as_str()))
            })
            .collect::<HashMap<_, _>>();
        let mut missions = rows
            .iter()
            .filter_map(|row| match row {
                SidebarRow::Mission { node, summary, .. } => summary
                    .mission
                    .linked_chat
                    .as_ref()
                    .filter(|chat| !chat.archived && summary.mission.archived_at.is_none())
                    .and_then(|chat| tab_of_session.get(chat.session_id.as_str()))
                    .map(|tab| (*tab, node)),
                SidebarRow::Tab { .. } => None,
            })
            .collect::<Vec<_>>();
        missions.sort_by_key(|(_, node)| (node.position, node.id.clone()));
        let mut groups = Self::default();
        for (tab, node) in missions {
            groups
                .by_tab
                .entry(tab.to_owned())
                .or_default()
                .push(node.id.clone());
            groups.head_of.insert(node.id.clone(), tab.to_owned());
        }
        groups
    }

    pub(super) fn missions(&self, tab_node_id: &str) -> &[String] {
        self.by_tab
            .get(tab_node_id)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(super) fn head(&self, mission_node_id: &str) -> Option<&str> {
        self.head_of.get(mission_node_id).map(String::as_str)
    }
}

pub(super) fn is_agent_chat(member: &SessionDisplay) -> bool {
    !Runtime::parse(&member.agent_runtime).is_some_and(Runtime::is_shell)
        && member.archived_at.is_none()
}

/// A mission row's trailing status from the daemon's state: Working, Needs
/// you, Failed, Interrupted, or the unread dot once Settled or Ended. An Ended
/// mission otherwise looks like any stopped mission row (Jason, 2026-11-03).
pub(super) fn phase_attention(phase: MissionPhase, unread: bool) -> AttentionState {
    match phase {
        MissionPhase::NeedsYou => AttentionState::NeedsYou,
        MissionPhase::Failed => AttentionState::Error,
        MissionPhase::Interrupted => AttentionState::Interrupted,
        MissionPhase::Working | MissionPhase::Unknown => AttentionState::Working,
        MissionPhase::Settled | MissionPhase::Ended if unread => AttentionState::Unread,
        MissionPhase::Settled | MissionPhase::Ended => AttentionState::None,
    }
}

/// The trailing element for a phase.
pub(super) fn phase_indicator(phase: MissionPhase, unread: bool) -> AnyElement {
    mission_attention_indicator(phase_attention(phase, unread))
}

/// A mission's state glyph: a mission that needs you shows the status UI's
/// raised hand, where a chat shows its alert.
pub(super) fn mission_attention_indicator(attention: AttentionState) -> AnyElement {
    if attention != AttentionState::NeedsYou {
        return super::elements::attention_indicator(attention);
    }
    gpui::div()
        .size(gpui::rems(12. / 16.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(
            gpui::svg()
                .path("hand.svg")
                .size(gpui::px(12.))
                .text_color(theme::warning()),
        )
        .into_any_element()
}

/// The order a collapsed group names its most urgent state in.
const URGENCY: [MissionPhase; 6] = [
    MissionPhase::NeedsYou,
    MissionPhase::Failed,
    MissionPhase::Interrupted,
    MissionPhase::Working,
    MissionPhase::Settled,
    MissionPhase::Ended,
];

/// The most urgent state among a group's missions, with how many are in it.
pub(super) fn most_urgent(phases: &[Option<MissionPhase>]) -> Option<(MissionPhase, usize)> {
    URGENCY.into_iter().find_map(|urgent| {
        let count = phases
            .iter()
            .filter(|phase| **phase == Some(urgent))
            .count();
        (count > 0).then_some((urgent, count))
    })
}

/// "2 missions · 1 needs you", the collapsed group's second line.
pub(super) fn collapsed_summary(phases: &[Option<MissionPhase>]) -> String {
    let missions = crate::i18n::tn("count.missions", phases.len());
    match most_urgent(phases) {
        Some((phase, count)) => format!(
            "{missions} · {}",
            crate::i18n::tf(
                match phase {
                    MissionPhase::NeedsYou => "format.link_group_needs_you",
                    MissionPhase::Failed => "format.link_group_failed",
                    MissionPhase::Interrupted => "format.link_group_interrupted",
                    MissionPhase::Working | MissionPhase::Unknown => "format.link_group_working",
                    MissionPhase::Settled => "format.link_group_settled",
                    MissionPhase::Ended => "format.link_group_ended",
                },
                &[("count", count.to_string())],
            )
        ),
        None => missions,
    }
}

/// Where a dragged mission is over a chat row: its top and bottom quarters
/// keep the reorder markers, its middle half links.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ChatRowZone {
    Before,
    Link,
    After,
}

pub(super) fn chat_row_zone(y: f32, top: f32, height: f32) -> ChatRowZone {
    let offset = (y - top) / height.max(1.);
    if offset < 0.25 {
        ChatRowZone::Before
    } else if offset > 0.75 {
        ChatRowZone::After
    } else {
        ChatRowZone::Link
    }
}

/// One agent chat the picker can offer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ChatChoice {
    pub(crate) session_id: String,
    pub(crate) title: String,
    pub(crate) runtime: String,
    /// When the chat last did anything, for the recency hint and order.
    pub(crate) active_at: Option<i64>,
    pub(crate) missions: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PickerEntry {
    Heading(&'static str),
    Chat(ChatChoice),
}

/// How many recently active chats the picker lists before anything is typed.
const RECENT_CHATS: usize = 5;

/// The picker's list: with nothing typed, the chats that already have linked
/// missions and then the most recently active others; typed text searches
/// every chat by title.
pub(crate) fn picker_entries(chats: &[ChatChoice], query: &str) -> Vec<PickerEntry> {
    let query = query.trim().to_lowercase();
    if !query.is_empty() {
        return chats
            .iter()
            .filter(|chat| chat.title.to_lowercase().contains(&query))
            .cloned()
            .map(PickerEntry::Chat)
            .collect();
    }
    let by_recency = |chats: &mut Vec<&ChatChoice>| {
        chats.sort_by(|a, b| b.active_at.cmp(&a.active_at).then(a.title.cmp(&b.title)))
    };
    let mut linked = chats.iter().filter(|chat| chat.missions > 0).collect();
    by_recency(&mut linked);
    let mut recent = chats.iter().filter(|chat| chat.missions == 0).collect();
    by_recency(&mut recent);
    let mut entries = Vec::new();
    if !linked.is_empty() {
        entries.push(PickerEntry::Heading("heading.with_missions"));
        entries.extend(linked.into_iter().cloned().map(PickerEntry::Chat));
    }
    if !recent.is_empty() {
        entries.push(PickerEntry::Heading("heading.recent"));
        entries.extend(
            recent
                .into_iter()
                .take(RECENT_CHATS)
                .cloned()
                .map(PickerEntry::Chat),
        );
    }
    entries
}

/// How a reorder drop changes a mission's link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum LinkChange {
    /// Into another group: link to that tab's chat.
    Link(String),
    /// Out of its group among the scope's own rows: unlink.
    Unlink,
}

/// A drop between a group's rows links the mission to the group's chat, a
/// grouped mission dropped among the scope's own rows leaves its chat, and a
/// move within its own group keeps the link.
pub(super) fn link_change(target_group: Option<&str>, head: Option<&str>) -> Option<LinkChange> {
    match (target_group, head) {
        (Some(tab), head) if head != Some(tab) => Some(LinkChange::Link(tab.to_owned())),
        (None, Some(_)) => Some(LinkChange::Unlink),
        _ => None,
    }
}

/// A drop between a several-chat group's rows: the move waits for the chat to be
/// picked, and is dropped with the picker.
#[derive(Clone)]
pub(super) struct PendingMove {
    pub(super) node: super::NodeRow,
    pub(super) project: Option<super::NodeRow>,
    pub(super) order: Vec<super::NodeRow>,
}

/// Where the picker opens: beside its menu row as a submenu, or at the pointer
/// after a drop.
#[derive(Clone, Copy, Debug)]
pub(crate) enum PickerAnchor {
    Pointer,
    Row(gpui::Bounds<gpui::Pixels>),
}

/// When a chat last did anything: its tab's last finished turn or visit, else
/// its process's start or stop.
pub(super) fn chat_active_at(
    entry: &runner_core::protocol::DirectSessionEntry,
    nodes: &[runner_core::protocol::node::NodeRow],
) -> Option<i64> {
    let tab = nodes.iter().find(|node| {
        super::PaneLayout::from_node_row(node)
            .is_ok_and(|layout| layout.session_ids().contains(&entry.session_id))
    });
    let stamp = |text: &Option<String>| {
        text.as_deref()
            .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
            .map(|at| at.timestamp_millis())
    };
    [
        tab.and_then(|tab| stamp(&tab.last_completed_at)),
        tab.and_then(|tab| stamp(&tab.last_viewed_at)),
        entry.started_at.map(|at| at.timestamp_millis()),
        entry.stopped_at.map(|at| at.timestamp_millis()),
    ]
    .into_iter()
    .flatten()
    .max()
}

/// A dragged mission held over the middle of a chat's tab row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LinkDrop {
    pub(super) mission_node: String,
    pub(super) tab_node: String,
    /// The tab's agent chats as (session, title).
    pub(super) chats: Vec<(String, String)>,
}

fn collect_pane_sessions(pane: &runner_core::companion::Pane, out: &mut Vec<String>) {
    match pane {
        runner_core::companion::Pane::Leaf {
            session: Some(session),
        } => out.push(session.clone()),
        runner_core::companion::Pane::Leaf { session: None }
        | runner_core::companion::Pane::Unknown(_) => {}
        runner_core::companion::Pane::Split { a, b, .. } => {
            collect_pane_sessions(a, out);
            collect_pane_sessions(b, out);
        }
    }
}

impl super::Sidebar {
    /// The agent chats a mission in `project_id` can link to: unarchived direct
    /// chats in the same project, or in none. `only` narrows them to a tab's.
    pub(super) fn chat_choices(
        &self,
        project_id: Option<&str>,
        only: Option<&[String]>,
        cx: &gpui::App,
    ) -> Vec<ChatChoice> {
        if self.front_page.is_remote() {
            let Some(overview) = self.remote_overview(cx) else {
                return Vec::new();
            };
            let mut missions = HashMap::<&str, usize>::new();
            for mission in &overview.missions {
                if let Some(chat) = &mission.linked_chat {
                    *missions.entry(chat.session_id.as_str()).or_default() += 1;
                }
            }
            let mut session_ids = Vec::new();
            for tab in &overview.tabs {
                if tab.project.as_deref() == project_id {
                    collect_pane_sessions(&tab.panes, &mut session_ids);
                }
            }
            let mut seen = std::collections::HashSet::new();
            return session_ids
                .into_iter()
                .filter(|session_id| {
                    seen.insert(session_id.clone())
                        && only.is_none_or(|only| only.contains(session_id))
                })
                .filter_map(|session_id| {
                    let session = overview.sessions.get(&session_id)?;
                    if session.kind != runner_core::companion::SessionKind::Chat {
                        return None;
                    }
                    let display = SessionDisplay::remote(&session_id, session, false, false, false);
                    Some(ChatChoice {
                        title: display.label(self.live_title(&session_id, cx).as_deref()),
                        runtime: session.runtime.clone().unwrap_or_default(),
                        active_at: None,
                        missions: missions
                            .get(session_id.as_str())
                            .copied()
                            .unwrap_or_default(),
                        session_id,
                    })
                })
                .collect();
        }
        let store = self.app_store.read(cx);
        let mut missions = HashMap::<&str, usize>::new();
        for summary in &store.missions {
            if let Some(chat) = summary
                .mission
                .linked_chat
                .as_ref()
                .filter(|_| summary.mission.archived_at.is_none())
            {
                *missions.entry(chat.session_id.as_str()).or_default() += 1;
            }
        }
        store
            .sessions
            .iter()
            .filter(|entry| {
                entry.archived_at.is_none()
                    && !Runtime::parse(&entry.agent_runtime).is_some_and(Runtime::is_shell)
                    && entry.project_id.as_deref() == project_id
                    && only.is_none_or(|only| only.contains(&entry.session_id))
            })
            .map(|entry| ChatChoice {
                session_id: entry.session_id.clone(),
                title: super::session_label_live(
                    entry,
                    self.live_title(&entry.session_id, cx).as_deref(),
                ),
                runtime: entry.agent_runtime.clone(),
                active_at: chat_active_at(entry, &store.nodes),
                missions: missions
                    .get(entry.session_id.as_str())
                    .copied()
                    .unwrap_or_default(),
            })
            .collect()
    }

    pub(super) fn open_link_picker(
        &mut self,
        mission_id: String,
        only: Option<Vec<String>>,
        pending: Option<PendingMove>,
        anchor: PickerAnchor,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let mission_info = if self.front_page.is_remote() {
            self.remote_overview(cx).and_then(|overview| {
                let mission = overview.missions.into_iter().find(|m| m.id == mission_id)?;
                Some((
                    mission.project,
                    mission.linked_chat.map(|chat| chat.session_id),
                ))
            })
        } else {
            self.app_store
                .read(cx)
                .missions
                .iter()
                .find(|summary| summary.mission.id == mission_id)
                .map(|summary| {
                    (
                        summary.mission.project_id.clone(),
                        summary
                            .mission
                            .linked_chat
                            .as_ref()
                            .map(|chat| chat.session_id.clone()),
                    )
                })
        };
        let Some((project_id, linked_session_id)) = mission_info else {
            return;
        };
        let chats = self.chat_choices(project_id.as_deref(), only.as_deref(), cx);
        let scope = project_id
            .as_deref()
            .and_then(|id| {
                self.source_projects(cx)
                    .into_iter()
                    .find(|project| project.id == id)
                    .map(|project| project.name)
            })
            .unwrap_or_else(|| crate::i18n::text("ui.recents_scope").to_owned());
        let (anchor, beside) = match anchor {
            PickerAnchor::Pointer => (
                gpui::Bounds::new(
                    window.mouse_position(),
                    gpui::size(gpui::px(0.), gpui::px(0.)),
                ),
                false,
            ),
            PickerAnchor::Row(bounds) => (bounds, true),
        };
        let root = cx.entity();
        // As a submenu, Escape returns to the menu it came from.
        let back_root = root.clone();
        let on_back: Option<runner_app::ui::menu::DismissHandler> = beside.then(|| {
            std::rc::Rc::new(move |window: &mut gpui::Window, cx: &mut gpui::App| {
                back_root.update(cx, |this, cx| {
                    this.close_link_picker(cx);
                    if let Some(menu) = &this.context_menu {
                        let focus = menu.read(cx).focus_handle();
                        focus.focus(window, cx);
                    }
                });
            }) as runner_app::ui::menu::DismissHandler
        });
        let pick_root = root.clone();
        let pick_mission = mission_id.clone();
        let picker = cx.new(move |picker_cx| {
            super::link_picker::LinkPicker::new(
                anchor,
                beside,
                on_back,
                scope,
                linked_session_id,
                chats,
                std::rc::Rc::new(move |session, window, cx| {
                    pick_root.update(cx, |this, cx| {
                        this.close_link_picker(cx);
                        this.close_context_menu(window, cx);
                        if this.front_page.is_remote() {
                            if let Some(shell) = this.shell.upgrade() {
                                let pending = pending.clone();
                                let pick_mission = pick_mission.clone();
                                shell.update(cx, |root, cx| {
                                    if let Some(pending) = pending {
                                        root.remote_client.remote_action(
                                            "sidebar-order".into(),
                                            &["linkMission", "move"],
                                            move |client| {
                                                client.link_mission(pick_mission, session)?;
                                                client.move_item(
                                                    &pending.node,
                                                    pending.project.as_ref(),
                                                    &pending.order,
                                                )?;
                                                Ok(Default::default())
                                            },
                                            cx,
                                        );
                                    } else {
                                        root.remote_client.remote_action(
                                            format!("mission:{pick_mission}"),
                                            &["linkMission"],
                                            move |client| {
                                                client.link_mission(pick_mission, session)?;
                                                Ok(Default::default())
                                            },
                                            cx,
                                        );
                                    }
                                });
                            }
                            return;
                        }
                        // A rejected link leaves the mission where it was.
                        if this.link_mission_to(pick_mission.clone(), session, cx) {
                            if let Some(pending) = &pending {
                                this.apply_pending_move(pending, cx);
                            }
                        }
                    });
                }),
                std::rc::Rc::new(move |window, cx| {
                    root.update(cx, |this, cx| {
                        this.close_link_picker(cx);
                        this.close_context_menu(window, cx);
                    });
                }),
                picker_cx,
            )
        });
        let focus = picker.read(cx).focus_handle(cx);
        focus.focus(window, cx);
        self.link_picker = Some(picker);
        self.schedule_shell_notify(cx);
        cx.notify();
    }

    fn apply_pending_move(&mut self, pending: &PendingMove, cx: &mut gpui::Context<Self>) {
        match self
            .front_page
            .move_item(&pending.node, pending.project.as_ref(), &pending.order)
        {
            Ok(nodes) => {
                if let (Some(nodes), Some(shell)) = (nodes, self.shell.upgrade()) {
                    shell.update(cx, |shell, cx| shell.replace_tab_rows(&nodes, cx));
                }
                self.refresh_store(crate::app_store::StoreRefreshKind::All, cx);
            }
            Err(error) => self.report_error(error.to_string(), cx),
        }
    }

    pub(super) fn close_context_menu(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(menu) = self.context_menu.take() {
            menu.update(cx, |menu, cx| menu.close(window, cx));
            self.schedule_shell_notify(cx);
        }
    }

    pub(super) fn close_link_picker(&mut self, cx: &mut gpui::Context<Self>) {
        if self.link_picker.take().is_some() {
            self.schedule_shell_notify(cx);
            cx.notify();
        }
    }

    pub(crate) fn link_picker(&self) -> Option<gpui::Entity<super::link_picker::LinkPicker>> {
        self.link_picker.clone()
    }

    pub(super) fn link_mission_to(
        &mut self,
        mission_id: String,
        session_id: String,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.front_page.is_remote() {
            if let Some(shell) = self.shell.upgrade() {
                let key = format!("mission:{mission_id}");
                shell.update(cx, |root, cx| {
                    root.remote_client.remote_action(
                        key,
                        &["linkMission"],
                        move |client| {
                            client.link_mission(mission_id, session_id)?;
                            Ok(Default::default())
                        },
                        cx,
                    );
                });
            }
            return false;
        }
        match self.front_page.link_mission(mission_id, session_id) {
            Ok(_) => {
                self.refresh_store(crate::app_store::StoreRefreshKind::All, cx);
                true
            }
            Err(error) => {
                self.report_error(error.to_string(), cx);
                false
            }
        }
    }

    pub(super) fn unlink_mission(&mut self, mission_id: String, cx: &mut gpui::Context<Self>) {
        if self.front_page.is_remote() {
            if let Some(shell) = self.shell.upgrade() {
                let key = format!("mission:{mission_id}");
                shell.update(cx, |root, cx| {
                    root.remote_client.remote_action(
                        key,
                        &["unlinkMission"],
                        move |client| {
                            client.unlink_mission(mission_id)?;
                            Ok(Default::default())
                        },
                        cx,
                    );
                });
            }
            return;
        }
        match self.front_page.unlink_mission(mission_id) {
            Ok(_) => self.refresh_store(crate::app_store::StoreRefreshKind::All, cx),
            Err(error) => self.report_error(error.to_string(), cx),
        }
    }
}

impl Sidebar {
    /// Whether a mission has output the person has not read.
    pub(super) fn mission_unread(&self, summary: &MissionSummary) -> bool {
        self.mission_status_rollup(summary)
            .entries
            .iter()
            .any(|(_, status)| status.unread_since.is_some())
    }

    /// The mission's attention from the daemon's state; an older daemon's
    /// missions fall back to their members' statuses.
    pub(super) fn mission_attention(&self, summary: &MissionSummary) -> AttentionState {
        let attention = match summary.phase {
            Some(phase) => phase_attention(phase, self.mission_unread(summary)),
            None => {
                super::rows::status_attention(self.mission_status_rollup(summary).priority_value())
            }
        };
        if self.archiving_missions.contains(&summary.mission.id) {
            attention.max(AttentionState::Working)
        } else {
            attention
        }
    }

    /// A mission row's trailing status: its state, with a click focusing the
    /// member that needs attention.
    pub(super) fn render_mission_attention(
        &self,
        summary: &MissionSummary,
        node_id: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rollup = self.mission_status_rollup(summary);
        let archiving = self.archiving_missions.contains(&summary.mission.id);
        let id = SharedString::from(format!("attention-{node_id}"));
        let Some(phase) = summary.phase.filter(|_| !archiving) else {
            return self.render_rollup_attention(
                rollup,
                Some(node_id.to_owned()),
                archiving,
                false,
                id,
                cx,
            );
        };
        let target = rollup
            .target()
            .map(|session| (node_id.to_owned(), session.to_owned()));
        gpui::div()
            .id(id)
            .debug_selector({
                let id = node_id.to_owned();
                move || format!("MISSION_STATE {id} {phase:?}")
            })
            .flex_none()
            .cursor_pointer()
            .child(phase_indicator(phase, self.mission_unread(summary)))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                if let Some((node_id, session)) = &target {
                    this.focus_status_target(node_id, session.clone(), window, cx);
                }
            }))
            .into_any_element()
    }

    /// One scope's rows, with each linked mission inside its chat's group and
    /// out of the scope's own list. Returns the elements and the ids a drop
    /// among the scope's own rows orders.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_scope(
        &self,
        scope: Vec<SidebarRow>,
        rows: &[SidebarRow],
        groups: &LinkGroups,
        project_id: Option<String>,
        drop_kind: DropKind,
        parent_id: Option<String>,
        tab_index_by_node: &HashMap<String, u8>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let flat = scope
            .into_iter()
            .filter(|row| groups.head(&row.node().id).is_none())
            .collect::<Vec<_>>();
        let visible = flat
            .iter()
            .map(|row| row.node().id.clone())
            .collect::<Vec<_>>();
        flat.into_iter()
            .map(|row| {
                let shortcut_index = tab_index_by_node.get(&row.node().id).copied();
                let missions = groups
                    .missions(&row.node().id)
                    .iter()
                    .filter_map(|id| rows.iter().find(|row| &row.node().id == id).cloned())
                    .collect::<Vec<_>>();
                if missions.is_empty() {
                    self.render_sidebar_row(
                        row,
                        project_id.clone(),
                        shortcut_index,
                        drop_kind,
                        parent_id.clone(),
                        visible.clone(),
                        cx,
                    )
                } else {
                    self.render_link_group(
                        row,
                        missions,
                        project_id.clone(),
                        shortcut_index,
                        drop_kind,
                        parent_id.clone(),
                        visible.clone(),
                        tab_index_by_node,
                        cx,
                    )
                }
            })
            .collect()
    }

    /// What the current reorder line says: Link to the chat whose group it sits
    /// in, or Unlink from the chat a grouped mission would leave.
    pub(super) fn drop_hint(
        &self,
        rows: &[SidebarRow],
        groups: &LinkGroups,
        cx: &gpui::App,
    ) -> Option<DropHint> {
        let dragged = self.dragged_id.as_deref()?;
        let target = self.drop_target.as_ref()?;
        if target.kind == DropKind::Project {
            return None;
        }
        let summary = rows.iter().find_map(|row| match row {
            SidebarRow::Mission { node, summary, .. } if node.id == dragged => Some(summary),
            _ => None,
        })?;
        match link_change(target.group.as_deref(), groups.head(dragged))? {
            LinkChange::Link(tab) => rows.iter().find_map(|row| match row {
                SidebarRow::Tab {
                    node,
                    layout,
                    members,
                    ..
                } if node.id == tab => {
                    Some(DropHint::Link(self.tab_link_label(layout, members, cx)))
                }
                _ => None,
            }),
            LinkChange::Unlink => summary
                .mission
                .linked_chat
                .as_ref()
                .map(|chat| DropHint::Unlink(chat.title.clone())),
        }
    }

    /// A chat's tab row heading its linked missions in one outline, with link
    /// lines from the chat to each mission; collapsed, one row naming the
    /// mission count and the most urgent state.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_link_group(
        &self,
        head: SidebarRow,
        missions: Vec<SidebarRow>,
        project_id: Option<String>,
        shortcut_index: Option<u8>,
        drop_kind: DropKind,
        parent_id: Option<String>,
        visible_ids: Vec<String>,
        tab_index_by_node: &HashMap<String, u8>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tab_id = head.node().id.clone();
        let collapsed = self
            .settings(cx)
            .sidebar_collapsed_link_groups
            .contains(&tab_id);
        let state = self.group_state(missions.iter().filter_map(|row| match row {
            SidebarRow::Mission { summary, .. } => Some(summary),
            SidebarRow::Tab { .. } => None,
        }));
        // The line starts at the mark as drawn: a pinned row's pin pushes it right, but
        // its rename field shows no pin.
        let renaming = self.rename.as_ref().is_some_and(|rename| {
            rename
                .target
                .matches(runner_core::protocol::node::NodeType::Tab, &tab_id)
        });
        let mark = HEAD_SHIFT + mark_x(head.node().pinned_position.is_some() && !renaming);
        let selected = self.tab_selected(&tab_id, cx)
            || missions.iter().any(|row| {
                matches!(row, SidebarRow::Mission { summary, .. }
                    if self.mission_selected(&summary.mission.id, cx))
            });
        let toggle = self.link_group_toggle(&tab_id, collapsed, false, cx);
        let after_marker = format!(
            "{}:{tab_id}:true",
            super::rows_render::marker_prefix(drop_kind)
        );
        let head = match head {
            SidebarRow::Tab {
                node,
                layout,
                members,
                attention,
            } => self.render_tab_row(
                node,
                layout,
                members,
                attention,
                shortcut_index,
                drop_kind,
                parent_id,
                visible_ids,
                super::rows_render::GroupHead {
                    attention: collapsed.then_some(state.attention),
                    links: (!collapsed).then_some(missions.len()),
                    summary: collapsed.then(|| state.summary(&tab_id, theme::text_caption())),
                    in_group: true,
                },
                cx,
            ),
            head => self.render_sidebar_row(
                head,
                project_id.clone(),
                shortcut_index,
                drop_kind,
                parent_id,
                visible_ids,
                cx,
            ),
        };
        // The head's box reaches the outline's inner edge; its own padding holds the chevron.
        let head_box = gpui::div().ml(rems(-ROW_REACH / 16.)).child(head);
        let head_row = gpui::div()
            .relative()
            .child(head_box)
            .children((!collapsed).then(|| head_stub(&tab_id, mark)))
            .child(toggle);
        let mut group = gpui::div()
            .id(SharedString::from(format!("link-group-{tab_id}")))
            .debug_selector({
                let id = tab_id.clone();
                move || format!("LINK_GROUP {id}")
            })
            .relative()
            .flex()
            .flex_col()
            .gap(rems(GROUP_GAP / 16.))
            .ml(rems(-GUTTER / 16.))
            .pl(rems(GUTTER / 16.))
            .pt(gpui::px(1.))
            .when(collapsed, |group| group.pb(gpui::px(1.)))
            .when(!collapsed, |group| group.pb(rems(2. / 16.)))
            .child(head_row);
        group = group_fill(group, selected);
        if collapsed {
            return group
                .child(group_outline(&tab_id, selected))
                .into_any_element();
        }
        let group_ids = missions
            .iter()
            .map(|row| row.node().id.clone())
            .collect::<Vec<_>>();
        let last = missions.len().saturating_sub(1);
        for (index, row) in missions.into_iter().enumerate() {
            let SidebarRow::Mission {
                node,
                summary,
                attention,
            } = row
            else {
                continue;
            };
            let shortcut_index = tab_index_by_node.get(&node.id).copied();
            let mission_parent = node.parent_id.clone();
            let element = self.render_mission_row(
                node,
                summary,
                attention,
                project_id.clone(),
                shortcut_index,
                DropKind::Leaf,
                mission_parent,
                group_ids.clone(),
                Some((tab_id.clone(), MissionShape::projects(mark, index == last))),
                cx,
            );
            group = group.child(linked_mission(
                element,
                &tab_id,
                index,
                mark,
                index == last,
                ROW_REACH,
            ));
        }
        let line_after = (self.drop_marker.as_deref() == Some(after_marker.as_str()))
            .then(|| super::rows_render::drop_line(self.drop_hint.as_ref(), true));
        group
            .child(group_outline(&tab_id, selected))
            .children(line_after.into_iter().flatten())
            .into_any_element()
    }
}

/// A group's missions summed up for its collapsed row.
pub(super) struct GroupState {
    phases: Vec<Option<MissionPhase>>,
    urgent: Option<MissionPhase>,
    /// The most urgent mission's indicator, shown on the collapsed chat row.
    pub(super) attention: AttentionState,
}

impl GroupState {
    /// "2 missions · 1 needs you", coloured by the most urgent state, at the size of
    /// the line it takes the place of.
    pub(super) fn summary(&self, tab_id: &str, size: gpui::Rems) -> AnyElement {
        let id = tab_id.to_owned();
        gpui::div()
            .debug_selector(move || format!("LINK_GROUP_SUMMARY {id}"))
            .min_w(gpui::px(0.))
            .truncate()
            .text_size(size)
            .text_color(match self.urgent {
                Some(MissionPhase::NeedsYou | MissionPhase::Interrupted) => theme::warning(),
                Some(MissionPhase::Failed) => theme::danger(),
                _ => theme::faint(),
            })
            .child(collapsed_summary(&self.phases))
            .into_any_element()
    }
}

impl Sidebar {
    pub(super) fn group_state<'a>(
        &self,
        missions: impl Iterator<Item = &'a MissionSummary>,
    ) -> GroupState {
        let missions = missions.collect::<Vec<_>>();
        let phases = missions
            .iter()
            .map(|summary| summary.phase)
            .collect::<Vec<_>>();
        let urgent = most_urgent(&phases).map(|(phase, _)| phase);
        let unread = missions.iter().any(|summary| self.mission_unread(summary));
        GroupState {
            attention: urgent
                .map(|phase| phase_attention(phase, unread))
                .unwrap_or_default(),
            phases,
            urgent,
        }
    }

    /// The chevron that collapses a group, 9 px inside the outline and at least 6 px
    /// before the chat row's first glyph, centred on the row. Projects and Active share
    /// one collapse state per group (Jason, 2026-11-03); Active's outline has no gutter.
    pub(super) fn link_group_toggle(
        &self,
        tab_id: &str,
        collapsed: bool,
        active: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let gutter = if active { 0. } else { GUTTER };
        let toggle_id = tab_id.to_owned();
        gpui::div()
            .id(SharedString::from(format!("link-group-toggle-{tab_id}")))
            .debug_selector({
                let id = tab_id.to_owned();
                move || format!("LINK_GROUP_TOGGLE {id}")
            })
            .absolute()
            .top_0()
            .bottom_0()
            .left(rems((CHEVRON_INSET - gutter + 1.) / 16.))
            .w(rems(CHEVRON / 16.))
            .flex()
            .items_center()
            .cursor_pointer()
            .child(
                gpui::svg()
                    .path(if collapsed {
                        "chevron-right.svg"
                    } else {
                        "chevron-down.svg"
                    })
                    .size(rems(CHEVRON / 16.))
                    .text_color(theme::faint()),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                let id = toggle_id.clone();
                this.update_app_settings(cx, true, move |settings| {
                    if !settings.sidebar_collapsed_link_groups.remove(&id) {
                        settings.sidebar_collapsed_link_groups.insert(id.clone());
                    }
                    true
                });
                cx.notify();
            }))
            .into_any_element()
    }
}

/// The Active view's linked group: the chat's row and its active missions in
/// one outline, joined by link lines, as in Projects; collapsed, the chat's row alone.
pub(super) fn active_link_group(
    tab_id: &str,
    head: AnyElement,
    toggle: AnyElement,
    missions: Vec<AnyElement>,
    selected: bool,
) -> AnyElement {
    let mark = ACTIVE_HEAD_SHIFT + mark_x(false);
    let last = missions.len().saturating_sub(1);
    let collapsed = missions.is_empty();
    group_fill(gpui::div(), selected)
        .debug_selector({
            let id = tab_id.to_owned();
            move || format!("ACTIVE_LINK_GROUP {id}")
        })
        .relative()
        .flex()
        .flex_col()
        .gap(rems(GROUP_GAP / 16.))
        .pt(gpui::px(1.))
        .when(collapsed, |group| group.pb(gpui::px(1.)))
        .when(!collapsed, |group| group.pb(rems(2. / 16.)))
        .child(
            gpui::div()
                .relative()
                .child(head)
                .children((!missions.is_empty()).then(|| head_stub(tab_id, mark)))
                .child(toggle),
        )
        .children(missions.into_iter().enumerate().map(|(index, mission)| {
            linked_mission(mission, tab_id, index, mark, index == last, 0.)
        }))
        .child(group_outline(tab_id, selected))
        .into_any_element()
}

/// Sidebar geometry, in 1/16 rem: a row's border and padding put its mark's centre
/// here, and a pin glyph before the mark moves it right.
fn mark_x(pinned: bool) -> f32 {
    if pinned {
        33.
    } else {
        17.
    }
}
/// A mission's flag sits 20 px right of the chat's mark: its row's own border and
/// padding (11) take up the rest of this indent after the mark's centre (17).
const MISSION_INDENT_FROM_MARK: f32 = 3.;
/// A branch stops this short of the mission's flag.
const BRANCH_GAP: f32 = 3.;
const GROUP_GAP: f32 = 2.;
const CHEVRON: f32 = 10.;
/// How far a group's outline reaches left of its rows, for its chevron: inside a
/// project's padding, clear of the project's rule, and into the sidebar's own.
const GUTTER: f32 = 6.;
/// The chevron sits this far inside the outline's inner edge (Jason, 2026-11-03).
const CHEVRON_INSET: f32 = 9.;
/// And at least this far before the chat row's first glyph, pin or mark.
const CHEVRON_GAP: f32 = 6.;
/// How far a group's rows reach left of the scope's rows, to the outline's inner edge.
const ROW_REACH: f32 = GUTTER - 1.;
/// A group head's own left padding: the shift, a row's usual padding, and that reach.
const HEAD_PAD: f32 = HEAD_SHIFT + 10. + ROW_REACH;
/// A group head's corners follow the outline's inner edge.
const HEAD_RADIUS: f32 = 5.;

/// A group head's row, renaming or not: the outline is its border, so it spans the
/// outline's inner width, holds the chevron in its own padding and follows its corners.
pub(super) fn head_shape(
    row: gpui::Stateful<gpui::Div>,
    collapsed: bool,
) -> gpui::Stateful<gpui::Div> {
    shape_row(row, true, collapsed, HEAD_PAD, HEAD_RADIUS)
}

/// The same in Active, whose head row lies under the outline itself.
pub(super) fn active_head_shape(
    row: gpui::Stateful<gpui::Div>,
    collapsed: bool,
) -> gpui::Stateful<gpui::Div> {
    shape_row(
        row,
        true,
        collapsed,
        ACTIVE_HEAD_SHIFT + 10.,
        HEAD_RADIUS + 1.,
    )
}

/// A linked mission's row, renaming or not: like its head it spans the outline's
/// inner width with no border of its own, so the indent to its flag is its own padding.
/// Only the group's last row rounds, into the outline's bottom corners.
#[derive(Clone, Copy)]
pub(super) struct MissionShape {
    pad: f32,
    radius: f32,
    last: bool,
}

impl MissionShape {
    /// Under a chat whose mark is at `mark`.
    pub(super) fn projects(mark: f32, last: bool) -> Self {
        Self {
            pad: mark + MISSION_INDENT_FROM_MARK + 10. + ROW_REACH,
            radius: HEAD_RADIUS,
            last,
        }
    }

    /// The same in Active, whose rows lie under the outline itself.
    pub(super) fn active(last: bool) -> Self {
        Self {
            pad: ACTIVE_HEAD_SHIFT + mark_x(false) + MISSION_INDENT_FROM_MARK + 10.,
            radius: HEAD_RADIUS + 1.,
            last,
        }
    }

    pub(super) fn apply(self, row: gpui::Stateful<gpui::Div>) -> gpui::Stateful<gpui::Div> {
        #[cfg(test)]
        let row = crate::theme_snapshot::note_border("LINKED_MISSION_ROW_STYLE", row);
        shape_row(row, false, self.last, self.pad, self.radius)
    }
}

fn shape_row(
    row: gpui::Stateful<gpui::Div>,
    round_top: bool,
    round_bottom: bool,
    pad: f32,
    radius: f32,
) -> gpui::Stateful<gpui::Div> {
    let corner = |round: bool| rems(if round { radius / 16. } else { 0. });
    row.pl(rems(pad / 16.))
        .rounded_tl(corner(round_top))
        .rounded_tr(corner(round_top))
        .rounded_bl(corner(round_bottom))
        .rounded_br(corner(round_bottom))
}
/// The chat row moves right to make that room, and its line and missions with it:
/// the outline's border, the inset, the chevron and the gap, less the 11 px a row's
/// own border and padding already leave before its first glyph.
const HEAD_SHIFT: f32 = 1. - GUTTER + CHEVRON_INSET + CHEVRON + CHEVRON_GAP - 11.;
/// Active's outline is the group's own edge, with no gutter.
const ACTIVE_HEAD_SHIFT: f32 = HEAD_SHIFT + GUTTER;

fn link_color() -> gpui::Hsla {
    theme::faint()
}

/// A selected group's faint fill, under its rows and inside its outline.
fn group_fill<E: Styled>(group: E, selected: bool) -> E {
    if !selected {
        return group;
    }
    group
        .rounded(rems(6. / 16.))
        .bg(theme::with_alpha(gpui::white(), 0.02))
}

/// The outline around a group, over its rows so it never moves them. While the
/// chat or one of its missions is selected it takes the selected rows' border.
fn group_outline(tab: &str, selected: bool) -> impl IntoElement {
    let tab = tab.to_owned();
    let outline = gpui::div()
        .absolute()
        .inset_0()
        .rounded(rems(6. / 16.))
        .border_1()
        .border_color(if selected {
            theme::chrome_selected_border()
        } else {
            theme::border()
        });
    #[cfg(test)]
    let outline = crate::theme_snapshot::record_border("LINK_GROUP_OUTLINE_STYLE", outline);
    outline.debug_selector(move || format!("LINK_GROUP_OUTLINE {tab}"))
}

/// The line from below the chat's mark to the bottom of its row, continued by
/// the first mission's line across the gap between them.
fn head_stub(tab: &str, mark: f32) -> impl IntoElement {
    let tab = tab.to_owned();
    gpui::div()
        .debug_selector(move || format!("LINK_STUB {tab}"))
        .absolute()
        .left(rems((mark - 0.5) / 16.))
        .top(relative(0.5))
        .bottom(rems(-GROUP_GAP / 16.))
        .w(gpui::px(1.))
        .flex()
        .flex_col()
        .child(gpui::div().flex_none().h(rems(8. / 16.)))
        .child(recorded_fill(
            "LINK_STUB_FILL",
            gpui::div().flex_1().bg(link_color()),
        ))
}

/// Tests read back a surface's fill by its name.
fn recorded_fill(id: &'static str, element: gpui::Div) -> gpui::Div {
    #[cfg(test)]
    return crate::theme_snapshot::record_fill(id, element);
    #[cfg(not(test))]
    {
        let _ = id;
        element
    }
}

/// One mission in its chat's group: its row reaches `reach` left to the outline, with
/// the line down past its flag and a branch ending before it, drawn over the row's
/// fill. The last mission's line curves into its branch.
fn linked_mission(
    row: AnyElement,
    tab: &str,
    index: usize,
    mark: f32,
    last: bool,
    reach: f32,
) -> impl IntoElement {
    let left = rems((mark - 0.5) / 16.);
    let branch = rems((MISSION_INDENT_FROM_MARK + 11. - BRANCH_GAP + 0.5) / 16.);
    let key = format!("{tab} {index}");
    let line = gpui::div()
        .absolute()
        .left(left)
        .top(rems(-GROUP_GAP / 16.))
        .border_color(link_color());
    gpui::div()
        .relative()
        .child(gpui::div().ml(rems(-reach / 16.)).child(row))
        .map(|wrapper| {
            if last {
                let key = key.clone();
                wrapper.child(
                    line.debug_selector(move || format!("LINK_BRANCH {key}"))
                        .bottom(relative(0.5))
                        .w(branch)
                        .border_l_1()
                        .border_b_1()
                        .rounded_bl(rems(6. / 16.)),
                )
            } else {
                let line_key = key.clone();
                wrapper
                    .child(
                        line.debug_selector(move || format!("LINK_LINE {line_key}"))
                            .bottom_0()
                            .border_l_1(),
                    )
                    .child(
                        gpui::div()
                            .debug_selector(move || format!("LINK_BRANCH {key}"))
                            .absolute()
                            .left(left)
                            .top(relative(0.5))
                            .w(branch)
                            .h(gpui::px(1.))
                            .bg(link_color()),
                    )
            }
        })
}
