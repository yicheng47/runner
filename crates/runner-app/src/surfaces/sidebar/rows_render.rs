use super::elements::attention_indicator;
use super::elements::chat_tab_row_active;
use super::elements::empty_sidebar_label;
use super::elements::pin_indicator;
use super::elements::project_row_action;
use super::elements::project_row_label;
use super::elements::session_label_live;
use super::elements::sidebar_icon;
use super::elements::sidebar_row_label;
use super::elements::sidebar_row_shell;
use super::elements::sidebar_row_trailing_slot;
use super::elements::sidebar_tab_target;
use super::elements::tab_shortcut_pill;
use super::menus::sidebar_tab_icon;

use super::*;
use crate::surfaces::sidebar_logic::{AttentionState, DropKind};
use crate::surfaces::*;
use crate::*;
use gpui::{svg, DragMoveEvent};
use runner_app::ui::Tooltip;
use runner_backend::ops::mission::MissionSummary;
use runner_backend::repo::node::{NodeRow, NodeType};

impl Sidebar {
    /// The title the session's program is reporting right now, read from the
    /// shell's attached terminal. `None` once a session has no live terminal —
    /// a stopped row keeps whatever label it resolved to without one.
    fn live_title(&self, session_id: &str, cx: &App) -> Option<String> {
        self.shell
            .upgrade()?
            .read(cx)
            .attached_title(session_id)
            .filter(|title| !title.is_empty())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_tab_row(
        &self,
        node: NodeRow,
        layout: PaneLayout,
        members: Vec<DirectSessionEntry>,
        attention: AttentionState,
        shortcut_index: Option<u8>,
        drop_kind: DropKind,
        parent_id: Option<String>,
        visible_ids: Vec<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = self.shell.upgrade().is_some_and(|shell| {
            let shell = shell.read(cx);
            chat_tab_row_active(&shell.route, shell.tabs.active_tab_id(), node.id.as_str())
        });
        let live = members
            .iter()
            .any(|member| member.status == SessionStatus::Running);
        let pane_count = layout.root.leaves().len();
        let label = layout.name.clone().unwrap_or_else(|| {
            // The first pane in layout order speaks for an unnamed tab: moving
            // focus inside a tab must never relabel it in the rail (#587).
            let first = layout
                .root
                .leaves()
                .first()
                .and_then(|pane| pane.session_id.clone());
            let leader = first
                .as_deref()
                .and_then(|id| members.iter().find(|member| member.session_id == id))
                .or_else(|| members.first());
            leader.map_or_else(String::new, |member| {
                session_label_live(member, self.live_title(&member.session_id, cx).as_deref())
            })
        });
        let renaming = self
            .rename
            .as_ref()
            .is_some_and(|rename| rename.target.matches(NodeType::Tab, &node.id));

        let target = sidebar_tab_target(&layout, &members);
        let click_tab = node.id.clone();
        let click_session = target.session_id.clone();
        let single_runtime = (pane_count == 1)
            .then(|| members.first().map(|member| member.agent_runtime.as_str()))
            .flatten();
        let leaf_icon = sidebar_tab_icon(pane_count, single_runtime);
        let menu_node = node.clone();
        let menu_layout = layout.clone();
        let menu_members = members.clone();
        let menu_root = cx.entity();
        let context_node = node.clone();
        let context_layout = layout.clone();
        let context_members = members.clone();
        let base = if renaming {
            self.render_inline_rename_row(
                NodeType::Tab,
                &node.id,
                None,
                (leaf_icon, live),
                attention,
                shortcut_index,
                active,
                cx,
            )
        } else {
            let show_shortcut = self.show_shortcut_pills && shortcut_index.is_some();
            let trailing = if show_shortcut {
                sidebar_row_trailing_slot()
                    .children(shortcut_index.map(|index| tab_shortcut_pill(index, active)))
                    .into_any_element()
            } else {
                let more_button = IconButton::new(
                    SharedString::from(format!("sidebar-tab-actions-{}", node.id)),
                    "more-horizontal.svg",
                )
                .size(IconButtonSize::Xs)
                .stop_click_propagation(true)
                .reveal_on_group_hover("sidebar-row-actions")
                .tooltip("More actions")
                .on_press(move |window, cx| {
                    let position = window.mouse_position();
                    menu_root.update(cx, |this, cx| {
                        this.open_tab_menu(
                            menu_node.clone(),
                            menu_layout.clone(),
                            menu_members.clone(),
                            position,
                            window,
                            cx,
                        )
                    });
                });
                sidebar_row_trailing_slot()
                    .child(more_button)
                    .into_any_element()
            };
            sidebar_row_shell(
                SharedString::from(format!("sidebar-tab-{}", node.id)),
                active,
                false,
            )
            .children(node.pinned_position.is_some().then(pin_indicator))
            .child(sidebar_icon(leaf_icon, live))
            .child(sidebar_row_label(label.clone(), active, false))
            .child(
                self.render_rollup_attention(
                    self.tab_status_rollup(&members, cx),
                    Some(node.id.clone()),
                    members
                        .iter()
                        .any(|member| self.archiving_sessions.contains(&member.session_id)),
                    SharedString::from(format!("attention-{}", node.id)),
                    cx,
                ),
            )
            .child(trailing)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.activate_sidebar_session(&click_tab, &click_session, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_tab_menu(
                        context_node.clone(),
                        context_layout.clone(),
                        context_members.clone(),
                        event.position,
                        window,
                        cx,
                    );
                }),
            )
            .into_any_element()
        };
        if renaming {
            return base;
        }
        self.decorate_draggable_row(base, &node, label, drop_kind, parent_id, visible_ids, cx)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_mission_row(
        &self,
        node: NodeRow,
        summary: MissionSummary,
        attention: AttentionState,
        project_id: Option<String>,
        shortcut_index: Option<u8>,
        drop_kind: DropKind,
        parent_id: Option<String>,
        visible_ids: Vec<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = self.shell.upgrade().is_some_and(|shell| {
            matches!(
                &shell.read(cx).route,
                AppRoute::Mission(active_id) if active_id == &summary.mission.id
            )
        });

        let renaming = self.rename.as_ref().is_some_and(|rename| {
            rename
                .target
                .matches(NodeType::Mission, &summary.mission.id)
        });
        let label = summary.mission.title.clone();
        let menu_node = node.clone();
        let menu_summary = summary.clone();
        let menu_root = cx.entity();
        let context_node = node.clone();
        let context_summary = summary.clone();
        let base = if renaming {
            self.render_inline_rename_row(
                NodeType::Mission,
                &summary.mission.id,
                None,
                ("flag.svg", summary.any_session_live),
                attention,
                shortcut_index,
                active,
                cx,
            )
        } else {
            let show_shortcut = self.show_shortcut_pills && shortcut_index.is_some();
            let trailing = if show_shortcut {
                sidebar_row_trailing_slot()
                    .children(shortcut_index.map(|index| tab_shortcut_pill(index, active)))
                    .into_any_element()
            } else {
                sidebar_row_trailing_slot()
                    .child(
                        IconButton::new(
                            SharedString::from(format!(
                                "sidebar-mission-actions-{}",
                                summary.mission.id
                            )),
                            "more-horizontal.svg",
                        )
                        .size(IconButtonSize::Xs)
                        .stop_click_propagation(true)
                        .reveal_on_group_hover("sidebar-row-actions")
                        .tooltip("More actions")
                        .on_press(move |window, cx| {
                            let position = window.mouse_position();
                            menu_root.update(cx, |this, cx| {
                                this.open_mission_menu(
                                    menu_node.clone(),
                                    menu_summary.clone(),
                                    position,
                                    window,
                                    cx,
                                )
                            });
                        }),
                    )
                    .into_any_element()
            };
            sidebar_row_shell(
                SharedString::from(format!("sidebar-mission-{}", summary.mission.id)),
                active,
                false,
            )
            .children(node.pinned_position.is_some().then(pin_indicator))
            .child(sidebar_icon("flag.svg", summary.any_session_live))
            .child(sidebar_row_label(label.clone(), active, false))
            .child(self.render_rollup_attention(
                self.mission_status_rollup(&summary),
                Some(node.id.clone()),
                self.archiving_missions.contains(&summary.mission.id),
                SharedString::from(format!("attention-{}", node.id)),
                cx,
            ))
            .child(trailing)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.active_project_id = project_id.clone();
                this.open_mission(summary.mission.id.clone(), window, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_mission_menu(
                        context_node.clone(),
                        context_summary.clone(),
                        event.position,
                        window,
                        cx,
                    );
                }),
            )
            .into_any_element()
        };
        if renaming {
            return base;
        }
        self.decorate_draggable_row(base, &node, label, drop_kind, parent_id, visible_ids, cx)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_project(
        &self,
        node: NodeRow,
        project: runner_backend::repo::project::ProjectRow,
        nested: Vec<SidebarRow>,
        attention: AttentionState,
        visible_projects: Vec<String>,
        tab_index_by_node: &HashMap<String, u8>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let collapsed = self
            .settings(cx)
            .sidebar_collapsed_projects
            .contains(&project.id);
        let live = nested.iter().any(SidebarRow::is_live);
        let selected = self.active_project_id.as_deref() == Some(project.id.as_str());
        let renaming = self
            .rename
            .as_ref()
            .is_some_and(|rename| rename.target.matches(NodeType::Project, &project.id));
        let create_menu_root = cx.entity();
        let create_project_id = project.id.clone();
        let menu_root = cx.entity();
        let menu_project = project.clone();
        let toggle_id = project.id.clone();
        let context_project = project.clone();
        let header = if renaming {
            self.render_inline_rename_row(
                NodeType::Project,
                &project.id,
                Some(if collapsed {
                    "chevron-right.svg"
                } else {
                    "chevron-down.svg"
                }),
                ("folder-code.svg", live),
                if collapsed {
                    attention
                } else {
                    AttentionState::None
                },
                None,
                selected,
                cx,
            )
        } else {
            sidebar_row_shell(
                SharedString::from(format!("sidebar-project-{}", project.id)),
                selected,
                false,
            )
            .child(
                svg()
                    .path(if collapsed {
                        "chevron-right.svg"
                    } else {
                        "chevron-down.svg"
                    })
                    .size(rems(12. / 16.))
                    .flex_none()
                    .text_color(if selected {
                        theme::text()
                    } else {
                        theme::muted()
                    })
                    .group_hover("sidebar-row-actions", |icon| icon.text_color(theme::text())),
            )
            .child(sidebar_icon("folder-code.svg", live))
            .child(
                Tooltip::new(
                    SharedString::from(format!("project-cwd-{}", project.id)),
                    project.cwd.clone(),
                    project_row_label(project.name.clone()),
                )
                .expand(),
            )
            .children(collapsed.then(|| {
                self.render_status_attention(
                    &nested,
                    SharedString::from(format!("project-attention-{}", project.id)),
                    cx,
                )
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(project_row_action(
                        SharedString::from(format!("sidebar-project-create-{}", project.id)),
                        "plus.svg",
                        12.,
                        "New in project",
                        move |window, cx| {
                            let position = window.mouse_position();
                            create_menu_root.update(cx, |this, cx| {
                                this.open_project_create_menu(
                                    create_project_id.clone(),
                                    position,
                                    window,
                                    cx,
                                )
                            });
                        },
                    ))
                    .child(project_row_action(
                        SharedString::from(format!("sidebar-project-actions-{}", project.id)),
                        "more-horizontal.svg",
                        14.,
                        "Project actions",
                        move |window, cx| {
                            let position = window.mouse_position();
                            menu_root.update(cx, |this, cx| {
                                this.open_project_menu(menu_project.clone(), position, window, cx)
                            });
                        },
                    )),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_project(&toggle_id, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_project_menu(context_project.clone(), event.position, window, cx);
                }),
            )
            .into_any_element()
        };
        let label = project.name.clone();
        let project_id = project.id.clone();
        let nested_ids = nested
            .iter()
            .map(|row| row.node().id.clone())
            .collect::<Vec<_>>();
        let has_nested = !nested.is_empty();
        let project_node_id = node.id.clone();
        let drag_node_id = node.id.clone();
        let drag_label = label.clone();
        let drag_root = cx.entity();
        let over_projects = visible_projects.clone();
        let over_project_id = node.id.clone();
        let over_children = nested_ids.clone();
        let mut header_wrap = crate::platform_ui::sidebar_row_wrapper(SharedString::from(format!(
            "project-drag-wrap-{}",
            node.id
        )))
        .child(header);
        if !renaming {
            header_wrap = header_wrap
                .on_drag(
                    SidebarNodeDrag {
                        node_id: drag_node_id,
                        label: drag_label,
                    },
                    move |drag: &SidebarNodeDrag, _, _, cx| {
                        drag_root.update(cx, |this, cx| {
                            this.start_sidebar_drag(drag.node_id.clone(), cx);
                        });
                        cx.new(|_| drag.clone())
                    },
                )
                .on_drag_move::<SidebarNodeDrag>(cx.listener(
                    move |this, event: &DragMoveEvent<SidebarNodeDrag>, _, cx| {
                        if !event.bounds.contains(&event.event.position) {
                            return;
                        }
                        let dragged = event.drag(cx).node_id.clone();
                        let dragged_type = this
                            .app_store
                            .read(cx)
                            .nodes
                            .iter()
                            .find(|node| node.id == dragged)
                            .map(|node| node.node_type);
                        if dragged_type == Some(NodeType::Project) {
                            let after = event.event.position.y > event.bounds.center().y;
                            this.update_row_drop_target(
                                &dragged,
                                DropKind::Project,
                                None,
                                &over_projects,
                                &over_project_id,
                                after,
                                format!("project:{}:{after}", over_project_id),
                                cx,
                            );
                        } else {
                            this.update_project_container_drop_target(
                                &dragged,
                                &project_node_id,
                                &over_children,
                                cx,
                            );
                        }
                    },
                ))
                .on_drop(cx.listener(|this, drag: &SidebarNodeDrag, _, cx| {
                    this.commit_sidebar_drop(&drag.node_id, cx);
                }));
        }
        let container_marker = format!("container:{}", node.id);
        let before_marker = format!("project:{}:false", node.id);
        let after_marker = format!("project:{}:true", node.id);
        if self.drop_marker.as_deref() == Some(container_marker.as_str()) {
            header_wrap = header_wrap
                .border_1()
                .border_color(theme::accent())
                .rounded_sm();
        } else if self.drop_marker.as_deref() == Some(before_marker.as_str()) {
            header_wrap = header_wrap.border_t_2().border_color(theme::accent());
        } else if self.drop_marker.as_deref() == Some(after_marker.as_str()) {
            header_wrap = header_wrap.border_b_2().border_color(theme::accent());
        }
        div()
            .flex()
            .flex_col()
            .gap(rems(2. / 16.))
            .child(header_wrap)
            .children((!collapsed).then(|| {
                div()
                    .ml_3()
                    .pl_2()
                    .flex()
                    .flex_col()
                    .gap(rems(2. / 16.))
                    .border_l_1()
                    .border_color(theme::border())
                    .children(if nested.is_empty() {
                        if self.dragged_id.is_some() {
                            vec![self.render_empty_project_drop_area(node.id.clone(), cx)]
                        } else {
                            vec![empty_sidebar_label("Empty")]
                        }
                    } else {
                        nested
                            .into_iter()
                            .map(|row| {
                                let shortcut_index = tab_index_by_node.get(&row.node().id).copied();
                                self.render_sidebar_row(
                                    row,
                                    Some(project_id.clone()),
                                    shortcut_index,
                                    DropKind::Leaf,
                                    Some(node.id.clone()),
                                    nested_ids.clone(),
                                    cx,
                                )
                            })
                            .collect()
                    })
                    .children((has_nested && self.dragged_id.is_some()).then(|| {
                        self.render_end_drop_divider(
                            DropKind::Leaf,
                            Some(node.id.clone()),
                            nested_ids,
                            cx,
                        )
                    }))
            }))
            .into_any_element()
    }

    fn render_empty_project_drop_area(
        &self,
        project_node_id: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let marker = format!("container:{project_node_id}");
        let active = self.drop_marker.as_deref() == Some(marker.as_str());
        let drop_project_id = project_node_id.clone();
        let mut area = div()
            .id(SharedString::from(format!(
                "sidebar-empty-project-drop-{project_node_id}"
            )))
            .relative()
            .min_h(rems(28. / 16.))
            .flex_none()
            .on_drag_move::<SidebarNodeDrag>(cx.listener(
                move |this, event: &DragMoveEvent<SidebarNodeDrag>, _, cx| {
                    if !event.bounds.contains(&event.event.position) {
                        return;
                    }
                    let dragged = event.drag(cx).node_id.clone();
                    this.update_project_container_drop_target(&dragged, &drop_project_id, &[], cx);
                },
            ))
            .on_drop(cx.listener(|this, drag: &SidebarNodeDrag, _, cx| {
                this.commit_sidebar_drop(&drag.node_id, cx);
            }));
        if active {
            area = area.border_t_2().border_color(theme::accent());
        }
        area.into_any_element()
    }

    pub(super) fn render_end_drop_divider(
        &self,
        kind: DropKind,
        parent_id: Option<String>,
        visible_ids: Vec<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hovered_id = visible_ids
            .last()
            .expect("end divider requires a visible row")
            .clone();
        let marker_prefix = match kind {
            DropKind::Pinned => "pinned",
            DropKind::Project => "project",
            DropKind::Leaf => "leaf",
        };
        let scope = parent_id.as_deref().unwrap_or("root");
        let marker = format!("end:{marker_prefix}:{scope}");
        let active = self.drop_marker.as_deref() == Some(marker.as_str());
        let drop_marker = marker.clone();
        let mut divider = div()
            .id(SharedString::from(format!(
                "sidebar-end-drop-{marker_prefix}-{scope}"
            )))
            .relative()
            .h(rems(6. / 16.))
            .flex_none()
            .on_drag_move::<SidebarNodeDrag>(cx.listener(
                move |this, event: &DragMoveEvent<SidebarNodeDrag>, _, cx| {
                    if !event.bounds.contains(&event.event.position) {
                        return;
                    }
                    let dragged = event.drag(cx).node_id.clone();
                    this.update_row_drop_target(
                        &dragged,
                        kind,
                        parent_id.as_deref(),
                        &visible_ids,
                        &hovered_id,
                        true,
                        drop_marker.clone(),
                        cx,
                    );
                },
            ))
            .on_drop(cx.listener(|this, drag: &SidebarNodeDrag, _, cx| {
                this.commit_sidebar_drop(&drag.node_id, cx);
            }));
        if active {
            divider = divider.border_t_2().border_color(theme::accent());
        }
        divider.into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn decorate_draggable_row(
        &self,
        row: AnyElement,
        node: &NodeRow,
        label: String,
        kind: DropKind,
        parent_id: Option<String>,
        visible_ids: Vec<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let drag = SidebarNodeDrag {
            node_id: node.id.clone(),
            label,
        };
        let drag_root = cx.entity();
        let hovered_id = node.id.clone();
        let marker_prefix = match kind {
            DropKind::Pinned => "pinned",
            DropKind::Project => "project",
            DropKind::Leaf => "leaf",
        };
        let marker_before = format!("{marker_prefix}:{}:false", node.id);
        let marker_after = format!("{marker_prefix}:{}:true", node.id);
        let active_before = self.drop_marker.as_deref() == Some(marker_before.as_str());
        let active_after = self.drop_marker.as_deref() == Some(marker_after.as_str());
        let mut wrapper = crate::platform_ui::sidebar_row_wrapper(SharedString::from(format!(
            "sidebar-drag-{}",
            node.id
        )))
        .cursor_move()
        .child(row)
        .on_drag(drag, move |drag: &SidebarNodeDrag, _, _, cx| {
            drag_root.update(cx, |this, cx| {
                this.start_sidebar_drag(drag.node_id.clone(), cx);
            });
            cx.new(|_| drag.clone())
        })
        .on_drag_move::<SidebarNodeDrag>(cx.listener(
            move |this, event: &DragMoveEvent<SidebarNodeDrag>, _, cx| {
                if !event.bounds.contains(&event.event.position) {
                    return;
                }
                let dragged = event.drag(cx).node_id.clone();
                let after = event.event.position.y > event.bounds.center().y;
                this.update_row_drop_target(
                    &dragged,
                    kind,
                    parent_id.as_deref(),
                    &visible_ids,
                    &hovered_id,
                    after,
                    format!("{marker_prefix}:{hovered_id}:{after}"),
                    cx,
                );
            },
        ))
        .on_drop(cx.listener(|this, drag: &SidebarNodeDrag, _, cx| {
            this.commit_sidebar_drop(&drag.node_id, cx);
        }));
        if active_before {
            wrapper = wrapper.border_t_2().border_color(theme::accent());
        } else if active_after {
            wrapper = wrapper.border_b_2().border_color(theme::accent());
        }
        wrapper.into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_inline_rename_row(
        &self,
        kind: NodeType,
        id: &str,
        disclosure_icon: Option<&'static str>,
        icon: (&'static str, bool),
        attention: AttentionState,
        shortcut_index: Option<u8>,
        shortcut_selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(rename) = self
            .rename
            .as_ref()
            .filter(|rename| rename.target.matches(kind, id))
        else {
            return div().into_any_element();
        };
        let (icon, icon_active) = icon;
        let input = rename.input.clone();
        sidebar_row_shell(
            SharedString::from(format!("sidebar-rename-{id}")),
            true,
            false,
        )
        .children(disclosure_icon.map(|icon| {
            svg()
                .path(icon)
                .size(rems(12. / 16.))
                .flex_none()
                .text_color(theme::text())
        }))
        .child(sidebar_icon(icon, icon_active))
        .child(div().min_w(px(0.)).flex_1().child(input))
        .child(attention_indicator(attention))
        .children(
            shortcut_index
                .filter(|_| self.show_shortcut_pills)
                .map(|index| {
                    sidebar_row_trailing_slot().child(tab_shortcut_pill(index, shortcut_selected))
                }),
        )
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
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
        .into_any_element()
    }
}
