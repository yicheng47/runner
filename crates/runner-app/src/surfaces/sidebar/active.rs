use super::elements::{
    sidebar_icon, sidebar_row_label, sidebar_row_shell, sidebar_row_trailing_slot,
    tab_shortcut_pill,
};
use super::rows_render::GroupHead;
use super::*;
use crate::surfaces::sidebar_logic::AttentionState;
use runner_app::ui::agent_status::StatusRollup;
use runner_core::protocol::model::Runtime;
use runner_core::protocol::status::{Activity, AgentStatus, Lifecycle, ObservationSource};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum ActiveGroup {
    /// Chats and missions together, in creation order (#748).
    Agents,
    Terminals,
}

#[derive(Clone)]
pub(super) struct ActiveSidebarRow {
    pub(super) source: SidebarRow,
    group: ActiveGroup,
    rollup: StatusRollup,
}

impl ActiveGroup {
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Agents => "agents",
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

    pub(super) fn shortcut_row(&self, nodes: &[impl Clone + Into<NodeRow>]) -> SidebarShortcutRow {
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
    session: &SessionDisplay,
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
    if Runtime::parse(&session.agent_runtime).is_some_and(Runtime::is_shell)
        && status.observation.program_status.is_none()
    {
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
            node.created_at
                .as_deref()
                .and_then(|created| chrono::DateTime::parse_from_rfc3339(created).ok()),
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
        let mission_members = if self.front_page.is_remote() {
            HashSet::new()
        } else {
            store
                .missions
                .iter()
                .flat_map(|summary| summary.session_statuses.iter().map(|(id, _)| id.as_str()))
                .collect::<HashSet<_>>()
        };
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
                    let is_shell = |session: &SessionDisplay| {
                        Runtime::parse(&session.agent_runtime).is_some_and(Runtime::is_shell)
                    };
                    let terminals_only = members.iter().all(|session| is_shell(session));
                    let entries = members
                        .iter()
                        .filter_map(|session| {
                            let mut status = active_session_status(
                                session,
                                self.session_status(&session.session_id, cx).as_ref(),
                                if self.front_page.is_remote() {
                                    None
                                } else {
                                    store.session_activity.get(&session.session_id)
                                },
                            );
                            if !terminals_only
                                && is_shell(session)
                                && status.observation.program_status.is_none()
                            {
                                return None;
                            }
                            if self.archiving_sessions.contains(&session.session_id) {
                                super::rows::mark_archiving_status(&mut status);
                            }
                            Some((session.session_id.clone(), status))
                        })
                        .collect();
                    (
                        if terminals_only {
                            ActiveGroup::Terminals
                        } else {
                            ActiveGroup::Agents
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
                        ActiveGroup::Agents,
                        self.mission_status_rollup(summary),
                        summary.mission.running && summary.any_session_live,
                    )
                }
            };
            let attention = match &source {
                SidebarRow::Mission { summary, .. } => self.mission_attention(summary),
                SidebarRow::Tab { .. } => super::rows::status_attention(rollup.priority_value()),
            };
            let active = attention >= AttentionState::Unread || live;
            rows.push((
                active,
                ActiveSidebarRow {
                    source,
                    group,
                    rollup,
                },
            ));
        }
        // A chat heads its linked group while it or any of its missions is
        // active, and a shown group keeps every linked mission, Ended included,
        // until it is archived.
        let sources = rows
            .iter()
            .map(|(_, row)| row.source.clone())
            .collect::<Vec<_>>();
        let groups = super::link::LinkGroups::of(&sources);
        let mut shown = rows
            .iter()
            .filter(|(active, _)| *active)
            .map(|(_, row)| row.source.node().id.clone())
            .collect::<HashSet<_>>();
        let heads = shown
            .iter()
            .filter_map(|id| groups.head(id).map(str::to_owned))
            .collect::<Vec<_>>();
        shown.extend(heads);
        for (_, row) in &rows {
            let id = &row.source.node().id;
            if groups.head(id).is_some_and(|head| shown.contains(head)) {
                shown.insert(id.clone());
            }
        }
        let mut rows = rows
            .into_iter()
            .filter(|(_, row)| shown.contains(&row.source.node().id))
            .map(|(_, row)| row)
            .collect::<Vec<_>>();
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
        let sources = rows
            .iter()
            .map(|row| row.source.clone())
            .collect::<Vec<_>>();
        let link_groups = super::link::LinkGroups::of(&sources);
        for (group, label) in [
            (ActiveGroup::Agents, "heading.agents"),
            (ActiveGroup::Terminals, "heading.terminals"),
        ] {
            let members = rows
                .iter()
                .enumerate()
                .filter(|(_, row)| row.group() == group)
                .filter(|(_, row)| link_groups.head(&row.source.node().id).is_none())
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
                                let head_id = row.source.node().id.clone();
                                let missions = link_groups
                                    .missions(&head_id)
                                    .iter()
                                    .filter_map(|id| {
                                        rows.iter()
                                            .enumerate()
                                            .find(|(_, row)| &row.source.node().id == id)
                                    })
                                    .collect::<Vec<_>>();
                                if missions.is_empty() {
                                    return self.render_active_row(
                                        row,
                                        self.tab_index_by_node.get(&head_id).copied(),
                                        rename_index == Some(index),
                                        GroupHead::default(),
                                        cx,
                                    );
                                }
                                // One collapse state per group, shared with Projects.
                                let collapsed = self
                                    .settings(cx)
                                    .sidebar_collapsed_link_groups
                                    .contains(&head_id);
                                let state =
                                    self.group_state(missions.iter().filter_map(|(_, row)| {
                                        match &row.source {
                                            SidebarRow::Mission { summary, .. } => Some(summary),
                                            SidebarRow::Tab { .. } => None,
                                        }
                                    }));
                                let head = self.render_active_row(
                                    row,
                                    self.tab_index_by_node.get(&head_id).copied(),
                                    rename_index == Some(index),
                                    GroupHead {
                                        attention: collapsed.then_some(state.attention),
                                        summary: collapsed
                                            .then(|| state.summary(&head_id, theme::text_meta())),
                                        in_group: true,
                                        ..GroupHead::default()
                                    },
                                    cx,
                                );
                                let selected = self.tab_selected(&head_id, cx)
                                    || missions.iter().any(|(_, row)| {
                                        matches!(&row.source, SidebarRow::Mission { summary, .. }
                                            if self.mission_selected(&summary.mission.id, cx))
                                    });
                                let missions = if collapsed {
                                    Vec::new()
                                } else {
                                    missions
                                        .into_iter()
                                        .map(|(index, row)| {
                                            self.render_active_row(
                                                row,
                                                self.tab_index_by_node
                                                    .get(&row.source.node().id)
                                                    .copied(),
                                                rename_index == Some(index),
                                                GroupHead::default(),
                                                cx,
                                            )
                                        })
                                        .collect()
                                };
                                super::link::active_link_group(
                                    &head_id,
                                    head,
                                    self.link_group_toggle(&head_id, collapsed, true, cx),
                                    missions,
                                    selected,
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

    /// `group`: set for the chat row heading a linked group, whose outline is its
    /// border; collapsed, its summary replaces the place line.
    fn render_active_row(
        &self,
        row: &ActiveSidebarRow,
        index: Option<u8>,
        renaming: bool,
        group: GroupHead,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let store = self.app_store.read(cx);
        let shortcut = row.shortcut_row(&self.source_nodes(cx));
        let active = match &shortcut.target {
            SidebarActivationTarget::Tab { tab_id, .. } => self.tab_selected(tab_id, cx),
            SidebarActivationTarget::Mission { mission_id, .. } => {
                self.mission_selected(mission_id, cx)
            }
        };
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
                    node_project_id(&self.source_nodes(cx), node)
                        .or_else(|| target.project_id.clone()),
                )
            }
            SidebarRow::Mission { node, summary, .. } => (
                summary.any_session_live,
                summary.mission.cwd.as_deref(),
                node_project_id(&self.source_nodes(cx), node)
                    .or_else(|| summary.mission.project_id.clone()),
            ),
        };
        let projects = self.source_projects(cx);
        let project = projects
            .iter()
            .find(|project| Some(&project.id) == project_id.as_ref());
        let location = if self.front_page.is_remote() {
            match (project, cwd) {
                (Some(project), Some(cwd)) => format!("{} › {}", project.name, cwd),
                (Some(project), None) => project.name.clone(),
                (None, Some(cwd)) => cwd.to_owned(),
                _ => String::new(),
            }
        } else {
            active_location(
                project.and_then(|p| p.local.as_ref()),
                cwd,
                store.home_dir.as_deref().and_then(|path| path.to_str()),
            )
        };
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
        } else if let Some(attention) = group.attention {
            sidebar_row_trailing_slot().child(super::rows_render::group_state_glyph(
                &shortcut.node_id,
                attention,
            ))
        } else if let SidebarRow::Mission { summary, node, .. } = row.source.clone() {
            sidebar_row_trailing_slot().child(self.render_mission_attention(&summary, &node.id, cx))
        } else {
            sidebar_row_trailing_slot().child(row.rollup.render_pane_with_elapsed(
                SharedString::from(format!("active-status-{}", shortcut.node_id)),
                !self.front_page.is_remote(),
            ))
        };
        let hover_id = shortcut.node_id.clone();
        let target = shortcut.target;
        let menu_row = row.clone();
        let row_id = SharedString::from(format!("active-{}", shortcut.node_id));
        let collapsed_head = group.summary.is_some();
        let second_line = match group.summary {
            Some(summary) => summary,
            None => div()
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
                .child(div().min_w(px(0.)).truncate().child(location))
                .into_any_element(),
        };
        sidebar_row_shell(row_id.clone(), active || renaming, false, !group.in_group)
            .gap_2()
            .rounded(rems(6. / 16.))
            .when(group.in_group, |row| {
                super::link::active_head_shape(row, collapsed_head)
            })
            .text_size(theme::text_ui())
            .child(super::elements::sidebar_mark(
                row_id,
                sidebar_icon(icon, live),
            ))
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
                    .child(second_line),
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
