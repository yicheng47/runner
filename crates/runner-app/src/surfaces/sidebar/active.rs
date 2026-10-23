use super::elements::{
    sidebar_icon, sidebar_row_label, sidebar_row_shell, sidebar_row_trailing_slot,
    tab_shortcut_pill,
};
use super::*;
use runner_app::ui::agent_status::StatusRollup;
use runner_core::protocol::model::{MissionStatus, Runtime};
use runner_core::protocol::status::{Activity, AgentStatus, Lifecycle, ObservationSource};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum ActiveGroup {
    Chats,
    Missions,
    Terminals,
}

#[derive(Clone)]
pub(super) struct ActiveSidebarRow {
    source: SidebarRow,
    group: ActiveGroup,
    rollup: StatusRollup,
}

impl ActiveGroup {
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Chats => "chats",
            Self::Missions => "missions",
            Self::Terminals => "terminals",
        }
    }
}

impl ActiveSidebarRow {
    pub(super) fn group(&self) -> ActiveGroup {
        self.group
    }

    pub(super) fn title_and_icon(
        &self,
        live_title: impl Fn(&str) -> Option<String>,
    ) -> (String, ChatIcon) {
        match &self.source {
            SidebarRow::Tab {
                layout, members, ..
            } => {
                let pane_count = layout.root.leaves().len();
                let single_runtime = (pane_count == 1)
                    .then(|| members.first().map(|member| member.agent_runtime.as_str()))
                    .flatten();
                (
                    super::elements::tab_label_live(layout, members, live_title),
                    super::menus::sidebar_tab_icon(pane_count, single_runtime),
                )
            }
            SidebarRow::Mission { summary, .. } => {
                (summary.mission.title.clone(), ChatIcon::mission())
            }
        }
    }

    pub(super) fn shortcut_row(&self, nodes: &[NodeRow]) -> SidebarShortcutRow {
        let mut row = self.source.shortcut_row(nodes);
        if let (SidebarActivationTarget::Tab { session_id, .. }, Some(target)) =
            (&mut row.target, self.rollup.target())
        {
            *session_id = target.to_owned();
        }
        row
    }
}

fn active_session_status(
    session: &DirectSessionEntry,
    status: Option<&AgentStatus>,
    activity: Option<&SessionActivityState>,
) -> AgentStatus {
    let mut status = status.cloned().unwrap_or(AgentStatus {
        lifecycle: Lifecycle::Running,
        ..Default::default()
    });
    match session.status {
        SessionStatus::Running => {}
        SessionStatus::Stopped => status.lifecycle = Lifecycle::Stopped,
        SessionStatus::Crashed => status.lifecycle = Lifecycle::Error,
    }
    if Runtime::parse(&session.agent_runtime).is_some_and(Runtime::is_shell) {
        status.observation.activity = if activity == Some(&SessionActivityState::Busy) {
            Activity::Working
        } else {
            Activity::Idle
        };
        status.observation.source = ObservationSource::Baseline;
    }
    status
}

fn sort_active_rows(rows: &mut [ActiveSidebarRow]) {
    rows.sort_by_cached_key(|row| {
        let node = row.source.node();
        (
            row.group(),
            chrono::DateTime::parse_from_rfc3339(&node.created_at).ok(),
            node.id.clone(),
        )
    });
}

pub(super) fn active_location(
    project: Option<&runner_core::protocol::project::ProjectRow>,
    cwd: Option<&str>,
    home: Option<&str>,
) -> String {
    let compact = |path: &str| {
        runner_app::ui::field::compact_path_candidates(path, home)
            .into_iter()
            .next()
            .unwrap_or_default()
    };
    let Some(project) = project else {
        return compact(cwd.unwrap_or_default());
    };
    let Some(cwd) = cwd else {
        return project.name.clone();
    };
    match std::path::Path::new(cwd).strip_prefix(&project.cwd) {
        Ok(relative) if relative.as_os_str().is_empty() => project.name.clone(),
        Ok(relative) => {
            let relative = relative.strip_prefix(".worktrees").unwrap_or(relative);
            format!("{} › {}", project.name, relative.display())
        }
        Err(_) => format!("{} › {}", project.name, compact(cwd)),
    }
}

impl Sidebar {
    pub(super) fn resolved_active_rows(&self, cx: &App) -> Vec<ActiveSidebarRow> {
        let Some(shell) = self.shell.upgrade() else {
            return Vec::new();
        };
        let shell = shell.read(cx);
        self.resolved_active_rows_for_tabs(&shell.tabs, cx)
    }

    pub(super) fn resolved_active_rows_for_tabs(
        &self,
        tabs: &TabSet,
        cx: &App,
    ) -> Vec<ActiveSidebarRow> {
        let store = self.app_store.read(cx);
        let mission_members = store
            .missions
            .iter()
            .flat_map(|summary| summary.session_statuses.iter().map(|(id, _)| id.as_str()))
            .collect::<HashSet<_>>();
        let mut rows = Vec::new();
        for source in self.resolved_sidebar_rows_for_tabs(tabs, cx) {
            let (group, rollup, live) = match &source {
                SidebarRow::Tab { members, .. } => {
                    let members = members
                        .iter()
                        .filter(|session| {
                            session.archived_at.is_none()
                                && !mission_members.contains(session.session_id.as_str())
                        })
                        .collect::<Vec<_>>();
                    if members.is_empty() {
                        continue;
                    }
                    let is_shell = |session: &DirectSessionEntry| {
                        Runtime::parse(&session.agent_runtime).is_some_and(Runtime::is_shell)
                    };
                    let terminals_only = members.iter().all(|session| is_shell(session));
                    let entries = members
                        .iter()
                        .filter(|session| terminals_only || !is_shell(session))
                        .map(|session| {
                            let mut status = active_session_status(
                                session,
                                store.session_statuses.get(&session.session_id),
                                store.session_activity.get(&session.session_id),
                            );
                            if self.archiving_sessions.contains(&session.session_id) {
                                super::rows::mark_archiving_status(&mut status);
                            }
                            (session.session_id.clone(), status)
                        })
                        .collect();
                    (
                        if terminals_only {
                            ActiveGroup::Terminals
                        } else {
                            ActiveGroup::Chats
                        },
                        StatusRollup { entries },
                        members
                            .iter()
                            .any(|session| session.status == SessionStatus::Running),
                    )
                }
                SidebarRow::Mission { summary, .. } => {
                    if summary.mission.archived_at.is_some() {
                        continue;
                    }
                    (
                        ActiveGroup::Missions,
                        self.mission_status_rollup(summary),
                        summary.mission.status == MissionStatus::Running
                            && summary.any_session_live,
                    )
                }
            };
            if rollup.priority_value() >= 2 || live {
                rows.push(ActiveSidebarRow {
                    source,
                    group,
                    rollup,
                });
            }
        }
        sort_active_rows(&mut rows);
        rows
    }

    pub(super) fn render_active_groups(
        &self,
        rows: Vec<ActiveSidebarRow>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut list = div().flex().flex_col();
        if rows.is_empty() {
            return list
                .child(
                    div()
                        .px_5()
                        .py_2()
                        .text_size(theme::text_body())
                        .text_color(theme::faint())
                        .child(crate::i18n::text("ui.nothing_active")),
                )
                .into_any_element();
        }
        let rename_index = rows.iter().position(|row| {
            self.rename
                .as_ref()
                .is_some_and(|rename| match &row.source {
                    SidebarRow::Tab { node, .. } => rename.target.matches(NodeType::Tab, &node.id),
                    SidebarRow::Mission { summary, .. } => rename
                        .target
                        .matches(NodeType::Mission, &summary.mission.id),
                })
        });
        for (group, label) in [
            (ActiveGroup::Chats, "heading.chats"),
            (ActiveGroup::Missions, "heading.missions"),
            (ActiveGroup::Terminals, "heading.terminals"),
        ] {
            let members = rows
                .iter()
                .enumerate()
                .filter(|(_, row)| row.group() == group)
                .collect::<Vec<_>>();
            if members.is_empty() {
                continue;
            }
            let key = group.key();
            let collapsed = self
                .settings(cx)
                .sidebar_collapsed_active_groups
                .contains(key);
            list = list.child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .mb_5()
                    .child(
                        div().px_5().pb(rems(6. / 16.)).flex().items_center().child(
                            div()
                                .id(SharedString::from(format!("toggle-active-{key}")))
                                .debug_selector(move || format!("ACTIVE_GROUP_TOGGLE {key}"))
                                .group("sidebar-active-group-toggle")
                                .min_w(px(0.))
                                .flex()
                                .items_center()
                                .gap(rems(6. / 16.))
                                .cursor_pointer()
                                .text_size(theme::text_caption())
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(theme::faint())
                                .hover(|header| header.text_color(theme::muted()))
                                .child(crate::i18n::text(label))
                                .child(members.len().to_string())
                                .child(
                                    gpui::svg()
                                        .flex_none()
                                        .path(if collapsed {
                                            "chevron-right.svg"
                                        } else {
                                            "chevron-down.svg"
                                        })
                                        .size(rems(10. / 16.))
                                        .text_color(theme::faint())
                                        .group_hover("sidebar-active-group-toggle", |icon| {
                                            icon.text_color(theme::muted())
                                        }),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.update_app_settings(cx, true, |settings| {
                                        if !settings.sidebar_collapsed_active_groups.remove(key) {
                                            settings
                                                .sidebar_collapsed_active_groups
                                                .insert(key.to_owned());
                                        }
                                        true
                                    });
                                    cx.notify();
                                })),
                        ),
                    )
                    .children((!collapsed).then(|| {
                        div()
                            .px_3()
                            .pt(rems(2. / 16.))
                            .flex()
                            .flex_col()
                            .gap(rems(2. / 16.))
                            .children(members.into_iter().map(|(index, row)| {
                                self.render_active_row(
                                    row,
                                    self.tab_index_by_node.get(&row.source.node().id).copied(),
                                    rename_index == Some(index),
                                    cx,
                                )
                            }))
                    })),
            );
        }
        list.into_any_element()
    }

    fn open_active_menu(
        &mut self,
        row: &ActiveSidebarRow,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match &row.source {
            SidebarRow::Tab {
                node,
                layout,
                members,
                ..
            } => self.open_tab_menu(
                node.clone(),
                layout.clone(),
                members.clone(),
                position,
                window,
                cx,
            ),
            SidebarRow::Mission { node, summary, .. } => {
                self.open_mission_menu(node.clone(), summary.clone(), position, window, cx)
            }
        }
    }

    fn render_active_row(
        &self,
        row: &ActiveSidebarRow,
        index: Option<u8>,
        renaming: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let store = self.app_store.read(cx);
        let shortcut = row.shortcut_row(&store.nodes);
        let active = self.shell.upgrade().is_some_and(|shell| {
            let shell = shell.read(cx);
            match &shortcut.target {
                SidebarActivationTarget::Tab { tab_id, .. } => {
                    super::elements::chat_tab_row_active(
                        &shell.route,
                        shell.tabs.active_tab_id(),
                        tab_id,
                    )
                }
                SidebarActivationTarget::Mission { mission_id, .. } => {
                    matches!(&shell.route, AppRoute::Mission(id) if id == mission_id)
                }
            }
        });
        let (label, icon) = row.title_and_icon(|id| self.live_title(id, cx));
        let (live, cwd, project_id) = match &row.source {
            SidebarRow::Tab {
                node,
                layout,
                members,
                ..
            } => {
                let target = super::elements::sidebar_tab_target(layout, members);
                (
                    members
                        .iter()
                        .any(|member| member.status == SessionStatus::Running),
                    target.cwd.as_deref(),
                    node_project_id(&store.nodes, node).or_else(|| target.project_id.clone()),
                )
            }
            SidebarRow::Mission { node, summary, .. } => (
                summary.any_session_live,
                summary.mission.cwd.as_deref(),
                node_project_id(&store.nodes, node).or_else(|| summary.mission.project_id.clone()),
            ),
        };
        let project = store
            .projects
            .iter()
            .find(|project| Some(&project.id) == project_id.as_ref());
        let location = active_location(
            project,
            cwd,
            store.home_dir.as_deref().and_then(|path| path.to_str()),
        );
        let title = if renaming {
            div()
                .debug_selector(|| format!("ACTIVE_RENAME {}", shortcut.node_id))
                .min_w(px(0.))
                .child(self.rename.as_ref().unwrap().input.clone())
                .into_any_element()
        } else {
            sidebar_row_label(label, active, false).into_any_element()
        };
        let index = index.filter(|index| {
            keymap::effective_binding(
                &format!("select-tab-{index}"),
                &store.settings.keymap_overrides,
            )
            .is_some()
        });
        let trailing = if self.show_shortcut_pills && index.is_some() {
            sidebar_row_trailing_slot()
                .children(index.map(|index| tab_shortcut_pill(index, active)))
        } else if !renaming && self.hovered_row.as_deref() == Some(shortcut.node_id.as_str()) {
            let owner = cx.entity();
            let menu_row = row.clone();
            sidebar_row_trailing_slot().child(
                IconButton::new(
                    SharedString::from(format!("active-actions-{}", shortcut.node_id)),
                    "more-horizontal.svg",
                )
                .size(IconButtonSize::Xs)
                .stop_click_propagation(true)
                .tooltip(crate::i18n::text("ui.more_actions"))
                .on_press(move |window, cx| {
                    owner.update(cx, |this, cx| {
                        this.open_active_menu(&menu_row, window.mouse_position(), window, cx)
                    });
                }),
            )
        } else {
            sidebar_row_trailing_slot().child(row.rollup.render_pane(SharedString::from(format!(
                "active-status-{}",
                shortcut.node_id
            ))))
        };
        let hover_id = shortcut.node_id.clone();
        let target = shortcut.target;
        let menu_row = row.clone();
        sidebar_row_shell(
            SharedString::from(format!("active-{}", shortcut.node_id)),
            active || renaming,
            false,
        )
        .gap_2()
        .rounded(rems(6. / 16.))
        .text_size(theme::text_ui())
        .child(sidebar_icon(icon, live))
        .child(
            div()
                .min_w(px(0.))
                .flex_1()
                .flex()
                .flex_col()
                .gap(rems(2. / 16.))
                .child(
                    div()
                        .debug_selector(|| format!("ACTIVE_TITLE {}", shortcut.node_id))
                        .child(title),
                )
                .child(
                    div()
                        .debug_selector(|| format!("ACTIVE_LOCATION {}", shortcut.node_id))
                        .min_w(px(0.))
                        .flex()
                        .items_center()
                        .gap(rems(6. / 16.))
                        .text_size(theme::text_meta())
                        .text_color(theme::faint())
                        .child(
                            gpui::svg()
                                .path("folder.svg")
                                .size(rems(10. / 16.))
                                .flex_none()
                                .text_color(theme::faint()),
                        )
                        .child(div().min_w(px(0.)).truncate().child(location)),
                ),
        )
        .child(trailing)
        .when(renaming, |row| {
            row.on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "enter" => {
                        cx.stop_propagation();
                        this.submit_sidebar_rename(window, cx);
                    }
                    "escape" => {
                        cx.stop_propagation();
                        this.cancel_sidebar_rename(window, cx);
                    }
                    _ => {}
                }
            }))
        })
        .when(!renaming, |row| {
            row.on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                this.set_row_hovered(&hover_id, *hovered, cx)
            }))
            .on_click(cx.listener(move |this, _, window, cx| match &target {
                SidebarActivationTarget::Tab { tab_id, session_id } => {
                    this.activate_sidebar_session(tab_id, session_id, window, cx)
                }
                SidebarActivationTarget::Mission {
                    mission_id,
                    project_id,
                } => {
                    this.active_project_id = project_id.clone();
                    this.open_mission(mission_id.clone(), window, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_active_menu(&menu_row, event.position, window, cx);
                }),
            )
        })
        .into_any_element()
    }
}
