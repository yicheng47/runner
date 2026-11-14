use super::elements::empty_sidebar_label;
use super::elements::launcher_tab_row;
use super::elements::section_title;
use super::elements::workspace_new_tab_row;
use super::elements::workspace_row;

use super::*;
use crate::surfaces::sidebar::logic::{
    attention_rollups, indicator_visible, take_drag_state, DropKind, SidebarTree,
};
use crate::*;

// Shared with the layout probe so the production flex constraints stay under test.
pub(crate) fn sidebar_scroll_frame() -> gpui::Div {
    div().relative().min_h(px(0.)).flex_1().flex().flex_col()
}

pub(crate) fn sidebar_scroll_container(
    id: &'static str,
    scroll: &gpui::ScrollHandle,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .relative()
        .min_h(px(0.))
        .flex_1()
        .flex()
        .flex_col()
        .overflow_y_scroll()
        .scrollbar_width(px(0.))
        .track_scroll(scroll)
}

pub(crate) fn sidebar_contents(
    workspace_rows: impl IntoIterator<Item = AnyElement>,
    scroll: AnyElement,
) -> gpui::Div {
    div()
        .min_h(px(0.))
        .flex_1()
        .flex()
        .flex_col()
        .overflow_hidden()
        .pb_3()
        .child(
            crate::platform_ui::sidebar_section().child(
                div()
                    .px_3()
                    .pb_1()
                    .flex()
                    .flex_col()
                    .gap(rems(2. / 16.))
                    .children(workspace_rows),
            ),
        )
        .child(
            div()
                .mx_4()
                .mb_4()
                .mt(rems(10. / 16.))
                .h(px(1.))
                .flex_none()
                .bg(theme::chrome_selected_border()),
        )
        .child(scroll)
}

impl Sidebar {
    /// Where the launcher page's row is drawn: where the page opened, whatever
    /// its picker or folder then do. With nowhere visible to draw it, the Active
    /// view or a collapsed section, New tab keeps the selection.
    pub(crate) fn launcher_row_scope(&self, cx: &App) -> Option<ProjectScope> {
        let shell = self.shell.upgrade()?;
        let shell = shell.read(cx);
        if !shell.launcher_page_visible() {
            return None;
        }
        let scope = ProjectScope::or_root(shell.launcher_page_project(cx));
        let settings = self.settings(cx);
        let drawn = !settings.sidebar_active_only
            && match &scope {
                ProjectScope::Project(id) => {
                    settings.sidebar_projects_open
                        && !settings.sidebar_collapsed_projects.contains(id)
                        && self
                            .source_projects(cx)
                            .iter()
                            .any(|project| &project.id == id)
                }
                _ => settings.sidebar_chats_open,
            };
        drawn.then_some(scope)
    }

    pub(super) fn launcher_row(&self, cx: &App) -> AnyElement {
        let closable = self
            .shell
            .upgrade()
            .is_some_and(|shell| shell.read(cx).launcher_page_closable(cx));
        launcher_tab_row(closable.then(|| self.shell.clone()))
    }

    pub(crate) fn render_sidebar_contents(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.drop_marker.is_some()
            && !indicator_visible(self.drop_marker.as_deref(), cx.has_active_drag())
        {
            tracing::debug!(
                target: "sidebar::drag",
                path = "render-backstop",
                window_id = self.window_id,
                dragged_id = ?self.dragged_id,
                marker = ?self.drop_marker,
                "clear inactive sidebar drop indicator before render"
            );
            take_drag_state(
                &mut self.dragged_id,
                &mut self.drop_target,
                &mut self.drop_marker,
            );
        }
        let route = self
            .shell
            .upgrade()
            .map(|shell| shell.read(cx).route.clone())
            .unwrap_or_default();
        let active_only = self.settings(cx).sidebar_active_only;
        let launcher_row = self.launcher_row_scope(cx);
        let mut scroll = sidebar_scroll_container("sidebar-node-scroll", &self.scroll).when(
            !active_only,
            |scroll| {
                scroll.on_drop(cx.listener(|this, drag: &SidebarNodeDrag, window, cx| {
                    this.commit_sidebar_drop(&drag.node_id, window, cx);
                }))
            },
        );
        if !active_only {
            let rows = self.resolved_sidebar_rows(cx);
            let groups = super::link::LinkGroups::of(&rows);
            self.drop_hint = self.drop_hint(&rows, &groups, cx);
            let tree = SidebarTree::from_local(&self.source_nodes(cx));
            let resolve = |id: &str| rows.iter().find(|row| row.node().id == id).cloned();
            let pinned = tree
                .pinned
                .iter()
                .filter_map(|node| resolve(&node.id))
                .collect::<Vec<_>>();
            let has_pinned = !pinned.is_empty();
            let root_rows = tree
                .children(None)
                .filter_map(|node| resolve(&node.id))
                .collect::<Vec<_>>();
            let project_nodes = tree
                .projects
                .iter()
                .filter_map(|project| {
                    self.source_nodes(cx)
                        .iter()
                        .find(|node| node.id == project.id)
                        .cloned()
                })
                .collect::<Vec<_>>();
            let tab_index_by_node = self.tab_index_by_node.clone();
            let rollups = attention_rollups(
                rows.iter()
                    .filter(|row| row.node().pinned_position.is_none())
                    .map(|row| (row.node().parent_id.clone(), row.attention())),
            );
            let pinned = pinned
                .into_iter()
                .filter(|row| groups.head(&row.node().id).is_none())
                .collect::<Vec<_>>();
            if !pinned.is_empty() {
                let visible = pinned
                    .iter()
                    .map(|row| row.node().id.clone())
                    .collect::<Vec<_>>();
                scroll = scroll.child(
                    div()
                        .flex()
                        .flex_col()
                        .child(section_title(crate::i18n::text("heading.pinned")))
                        .child(
                            div()
                                .px_3()
                                .pt_1()
                                .flex()
                                .flex_col()
                                .gap(rems(2. / 16.))
                                .children(pinned.into_iter().map(|row| {
                                    let project_id =
                                        node_project_id(&self.source_nodes(cx), row.node());
                                    let shortcut_index =
                                        tab_index_by_node.get(&row.node().id).copied();
                                    let missions = groups
                                        .missions(&row.node().id)
                                        .iter()
                                        .filter_map(|id| {
                                            rows.iter().find(|row| &row.node().id == id).cloned()
                                        })
                                        .collect::<Vec<_>>();
                                    if missions.is_empty() {
                                        self.render_sidebar_row(
                                            row,
                                            project_id,
                                            shortcut_index,
                                            DropKind::Pinned,
                                            None,
                                            visible.clone(),
                                            cx,
                                        )
                                    } else {
                                        self.render_link_group(
                                            row,
                                            missions,
                                            project_id,
                                            shortcut_index,
                                            DropKind::Pinned,
                                            None,
                                            visible.clone(),
                                            &tab_index_by_node,
                                            cx,
                                        )
                                    }
                                }))
                                .children(self.dragged_id.is_some().then(|| {
                                    self.render_end_drop_divider(
                                        DropKind::Pinned,
                                        None,
                                        visible,
                                        cx,
                                    )
                                })),
                        ),
                );
            }

            let projects_open = self.settings(cx).sidebar_projects_open;
            let project_header_root = cx.entity();
            let project_add_root = project_header_root.clone();
            scroll = scroll.child(
                div()
                    .mt(if has_pinned {
                        rems(20. / 16.)
                    } else {
                        rems(0.)
                    })
                    .flex()
                    .flex_col()
                    .child(self.render_section_header(
                        crate::i18n::text("heading.projects"),
                        projects_open,
                        (!projects_open).then(|| {
                            self.render_status_attention(
                                rows.iter().filter(|row| row.node().parent_id.is_some()),
                                "project-section-attention".into(),
                                cx,
                            )
                        }),
                        crate::i18n::text("ui.add_project"),
                        move |this, cx| {
                            let open = !this.settings(cx).sidebar_projects_open;
                            this.update_app_settings(cx, true, |settings| {
                                settings.sidebar_projects_open = open;
                                true
                            });
                            cx.notify();
                        },
                        move |window, cx| {
                            project_add_root
                                .update(cx, |this, cx| this.open_project_modal(window, cx));
                        },
                        cx,
                    ))
                    .children(projects_open.then(|| {
                        let visible_projects = project_nodes
                            .iter()
                            .map(|node| node.id.clone())
                            .collect::<Vec<_>>();
                        let has_projects = !project_nodes.is_empty();
                        div()
                            .px_3()
                            .pt_1()
                            .flex()
                            .flex_col()
                            .gap(rems(2. / 16.))
                            .children(if project_nodes.is_empty() {
                                vec![empty_sidebar_label(crate::i18n::text("ui.no_projects_yet"))]
                            } else {
                                project_nodes
                                    .into_iter()
                                    .filter_map(|node| {
                                        let project = self
                                            .source_projects(cx)
                                            .iter()
                                            .find(|project| {
                                                node.ref_id.as_deref() == Some(project.id.as_str())
                                            })?
                                            .clone();
                                        let nested = tree
                                            .children(Some(&node.id))
                                            .filter_map(|node| resolve(&node.id))
                                            .collect();
                                        let attention = rollups
                                            .get(&Some(node.id.clone()))
                                            .copied()
                                            .unwrap_or_default();
                                        Some(self.render_project(
                                            node,
                                            project,
                                            nested,
                                            attention,
                                            visible_projects.clone(),
                                            &tab_index_by_node,
                                            launcher_row.as_ref(),
                                            &rows,
                                            &groups,
                                            cx,
                                        ))
                                    })
                                    .collect()
                            })
                            .children((has_projects && self.dragged_id.is_some()).then(|| {
                                self.render_end_drop_divider(
                                    DropKind::Project,
                                    None,
                                    visible_projects,
                                    cx,
                                )
                            }))
                    })),
            );

            let chats_open = self.settings(cx).sidebar_chats_open;
            let create_root = cx.entity();
            let create = IconButton::new("sidebar-create", "plus.svg")
                .size(IconButtonSize::Sm)
                .on_press(move |window, cx| {
                    create_root.update(cx, |this, cx| {
                        this.open_launcher_in(ProjectScope::Root, window, cx)
                    });
                });
            scroll = scroll.child(
                div()
                    .mt_5()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(sidebar_section_header(
                        "toggle-chats-section".into(),
                        crate::i18n::text("heading.recents"),
                        chats_open,
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(6. / 16.))
                            .children((!chats_open).then(|| {
                                self.render_status_attention(
                                    &root_rows,
                                    "recent-section-attention".into(),
                                    cx,
                                )
                            }))
                            .child(create)
                            .into_any_element(),
                        cx.listener(|this, _, _, cx| {
                            let open = !this.settings(cx).sidebar_chats_open;
                            this.update_app_settings(cx, true, |settings| {
                                settings.sidebar_chats_open = open;
                                true
                            });
                            cx.notify();
                        }),
                    ))
                    .children(chats_open.then(|| {
                        let visible = root_rows
                            .iter()
                            .map(|row| row.node().id.clone())
                            .filter(|id| groups.head(id).is_none())
                            .collect::<Vec<_>>();
                        let has_rows = !visible.is_empty();
                        let launcher_here = launcher_row == Some(ProjectScope::Root);
                        div()
                            .id("root-chat-scope")
                            .flex_1()
                            .px_3()
                            .pt_1()
                            .flex()
                            .flex_col()
                            .gap(rems(2. / 16.))
                            .children(launcher_here.then(|| self.launcher_row(cx)))
                            .children(if launcher_here && root_rows.is_empty() {
                                Vec::new()
                            } else if root_rows.is_empty() {
                                vec![empty_sidebar_label(crate::i18n::text("ui.no_chats_yet"))]
                            } else {
                                self.render_scope(
                                    root_rows,
                                    &rows,
                                    &groups,
                                    None,
                                    DropKind::Leaf,
                                    None,
                                    &tab_index_by_node,
                                    cx,
                                )
                            })
                            .children((has_rows && self.dragged_id.is_some()).then(|| {
                                self.render_end_drop_divider(DropKind::Leaf, None, visible, cx)
                            }))
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(
                                    move |this, event: &gpui::MouseDownEvent, window, cx| {
                                        cx.stop_propagation();
                                        this.open_sidebar_context_menu(
                                            event.position,
                                            vec![(
                                                UiMenuItem::new(crate::i18n::message("ui.new_tab"))
                                                    .icon("message-square-plus.svg"),
                                                SidebarMenuAction::NewChat(ProjectScope::Root),
                                            )],
                                            window,
                                            cx,
                                        );
                                    },
                                ),
                            )
                    })),
            );
        } else {
            let active_rows = self.resolved_active_rows(cx);
            scroll = scroll.child(self.render_active_groups(active_rows, cx));
        }
        let workspace_rows = WORKSPACE_ENTRIES
            .into_iter()
            .filter(|entry| {
                !self.front_page.is_remote()
                    || match entry {
                        WorkspaceEntry::NewTab => {
                            self.front_page.supports(&["catalog", "startChat"])
                        }
                        WorkspaceEntry::Role | WorkspaceEntry::Crew => {
                            self.front_page.supports(&["catalog"])
                        }
                        // Usage is read from the local daemon only.
                        WorkspaceEntry::Usage | WorkspaceEntry::MissionControl => false,
                    }
            })
            .map(|entry| {
                let active = if self.front_page.is_remote() {
                    matches!(
                        (entry, &self.remote_selection),
                        (
                            WorkspaceEntry::NewTab,
                            Some(crate::surfaces::remote_client::RemoteItem::Launcher)
                        ) | (
                            WorkspaceEntry::Role,
                            Some(crate::surfaces::remote_client::RemoteItem::Roles)
                        ) | (
                            WorkspaceEntry::Crew,
                            Some(crate::surfaces::remote_client::RemoteItem::Crews)
                        )
                    )
                } else {
                    entry.selected(&route)
                };
                let active = if entry == WorkspaceEntry::NewTab {
                    (active
                        || self
                            .shell
                            .upgrade()
                            .is_some_and(|shell| shell.read(cx).launcher_page_visible()))
                        && launcher_row.is_none()
                } else {
                    active
                };
                match entry {
                    WorkspaceEntry::NewTab => {
                        let shell = self.shell.clone();
                        let shortcut = keymap::effective_binding(
                            "new-chat",
                            &self.app_store.read(cx).settings.keymap_overrides,
                        )
                        .map(|combo| keymap::format_combo(&combo));
                        workspace_new_tab_row(shortcut, true, active, move |window, cx| {
                            if let Some(shell) = shell.upgrade() {
                                shell.update(cx, |shell, shell_cx| {
                                    shell.open_new_tab_modal(&NewTab, window, shell_cx);
                                });
                            }
                        })
                    }
                    WorkspaceEntry::Role => workspace_row(
                        "workspace-role",
                        "user.svg",
                        crate::i18n::text("noun.role"),
                        active,
                        true,
                        {
                            let shell = self.shell.clone();
                            move |window, cx| {
                                if let Some(shell) = shell.upgrade() {
                                    shell.update(cx, |shell, shell_cx| {
                                        shell.open_roles(window, shell_cx)
                                    });
                                }
                            }
                        },
                    ),
                    WorkspaceEntry::Crew => workspace_row(
                        "workspace-crew",
                        "users.svg",
                        crate::i18n::text("noun.crew"),
                        active,
                        true,
                        {
                            let shell = self.shell.clone();
                            move |window, cx| {
                                if let Some(shell) = shell.upgrade() {
                                    shell.update(cx, |shell, shell_cx| {
                                        shell.open_crews(window, shell_cx)
                                    });
                                }
                            }
                        },
                    ),
                    WorkspaceEntry::Usage => workspace_row(
                        "workspace-usage",
                        "gauge.svg",
                        crate::i18n::text("ui.usage_page_title"),
                        active,
                        true,
                        {
                            let shell = self.shell.clone();
                            move |window, cx| {
                                if let Some(shell) = shell.upgrade() {
                                    shell.update(cx, |shell, shell_cx| {
                                        shell.open_usage(window, shell_cx)
                                    });
                                }
                            }
                        },
                    ),
                    WorkspaceEntry::MissionControl => workspace_row(
                        "workspace-mission-control",
                        "users.svg",
                        "Mission Control",
                        active,
                        true,
                        {
                            let shell = self.shell.clone();
                            move |window, cx| {
                                if let Some(shell) = shell.upgrade() {
                                    shell.update(cx, |shell, shell_cx| {
                                        shell.open_page_route(crate::AppRoute::MissionControl, window, shell_cx)
                                    });
                                }
                            }
                        },
                    ),
                }
            });
        sidebar_contents(
            workspace_rows,
            sidebar_scroll_frame()
                .child(scroll)
                .child(self.scrollbar.clone())
                .into_any_element(),
        )
        .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_section_header<F, G>(
        &self,
        label: &'static str,
        open: bool,
        attention: Option<AnyElement>,
        plus_title: &'static str,
        on_toggle: F,
        on_plus: G,
        cx: &mut Context<Self>,
    ) -> AnyElement
    where
        F: Fn(&mut Sidebar, &mut Context<Sidebar>) + 'static,
        G: Fn(&mut Window, &mut App) + 'static,
    {
        let project_section = label == crate::i18n::text("heading.projects");
        let can_add = !self.front_page.is_remote()
            || if project_section {
                self.front_page.supports(&["folders", "createProject"])
            } else {
                self.front_page.supports(&["catalog", "startChat"])
                    || self.front_page.supports(&["startShell", "layout"])
            };
        let root = cx.weak_entity();
        sidebar_section_header(
            format!("toggle-{label}").into(),
            label,
            open,
            div()
                .flex()
                .items_center()
                .gap(rems(6. / 16.))
                .children(attention)
                .children(can_add.then(|| {
                    let button =
                        IconButton::new(SharedString::from(format!("add-{label}")), "plus.svg")
                            .size(IconButtonSize::Sm)
                            .tooltip(plus_title)
                            .on_press(on_plus)
                            .into_any_element();
                    if self.front_page.is_remote() && project_section {
                        div()
                            .debug_selector(|| "SIDEBAR_NEW_PROJECT".into())
                            .relative()
                            .flex_none()
                            .child(button)
                            .child(
                                gpui::canvas(
                                    move |bounds, _, cx| {
                                        let root = root.clone();
                                        cx.defer(move |cx| {
                                            let _ = root.update(cx, |sidebar, _| {
                                                sidebar.project_anchor = Some(bounds)
                                            });
                                        });
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .inset_0(),
                            )
                            .into_any_element()
                    } else {
                        button
                    }
                }))
                .into_any_element(),
            cx.listener(move |this, _, _, cx| on_toggle(this, cx)),
        )
        .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_sidebar_row(
        &self,
        row: SidebarRow,
        project_id: Option<String>,
        shortcut_index: Option<u8>,
        drop_kind: DropKind,
        parent_id: Option<String>,
        visible_ids: Vec<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        #[cfg(test)]
        crate::render_counts::count_render(&format!("sidebar-row:{}", row.node().id), cx);
        match row {
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
                Default::default(),
                cx,
            ),
            SidebarRow::Mission {
                node,
                summary,
                attention,
            } => self.render_mission_row(
                node,
                summary,
                attention,
                project_id,
                shortcut_index,
                drop_kind,
                parent_id,
                visible_ids,
                None,
                cx,
            ),
        }
    }
}
