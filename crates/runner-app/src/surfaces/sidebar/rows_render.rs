use super::elements::attention_indicator;
use super::elements::empty_sidebar_label;
use super::elements::project_row_action;
use super::elements::project_row_label;
use super::elements::sidebar_icon;
use super::elements::sidebar_row_shell;
use super::elements::sidebar_row_trailing_slot;
use super::elements::sidebar_tab_target;
use super::elements::tab_label_live;
use super::elements::tab_shortcut_pill;
use super::menus::sidebar_tab_icon;
use super::menus::MenuOrigin;

use super::*;
use crate::surfaces::sidebar::logic::{AttentionState, DropKind};
use crate::*;
use gpui::{svg, DragMoveEvent};
use runner_app::ui::Tooltip;

use runner_core::protocol::node::NodeType;

/// A tab row heading a linked group (#748): collapsed, it shows its most urgent
/// mission instead of the chat's own status and the group's summary under its
/// title; expanded, how many missions it links. Its box spans the group's inner
/// width, with the chevron inside its own left padding and no border of its own.
#[derive(Default)]
pub(super) struct GroupHead {
    pub(super) attention: Option<AttentionState>,
    pub(super) links: Option<usize>,
    pub(super) summary: Option<AnyElement>,
    pub(super) in_group: bool,
}

/// A group head's count of linked missions, shown while the group is expanded.
fn link_count(count: usize) -> gpui::Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(rems(3. / 16.))
        .text_size(theme::text_caption())
        .text_color(theme::faint())
        .child(
            svg()
                .path("link.svg")
                .size(rems(10. / 16.))
                .text_color(theme::faint()),
        )
        .child(count.to_string())
}

/// A collapsed group head's glyph for its most urgent mission.
pub(super) fn group_state_glyph(id: &str, attention: AttentionState) -> gpui::Div {
    let id = id.to_owned();
    div()
        .debug_selector(move || format!("LINK_GROUP_STATE {id} {attention:?}"))
        .child(super::link::mission_attention_indicator(attention))
}

/// The accent label a drop shows: Link to <chat> inside a chat row, or on a
/// reorder line whose drop would link or unlink the dragged mission.
fn drop_pill(icon: &'static str, label: String) -> gpui::Div {
    div()
        .flex_none()
        .px(rems(6. / 16.))
        .py(rems(1. / 16.))
        .flex()
        .items_center()
        .gap(rems(4. / 16.))
        .rounded_full()
        .bg(theme::accent())
        .text_size(theme::text_caption())
        .text_color(theme::accent_ink())
        .child(
            svg()
                .path(icon)
                .size(rems(10. / 16.))
                .text_color(theme::accent_ink()),
        )
        .child(label)
}

/// A reorder line as an element of its own, with its link hint centred on it.
/// The hint is drawn in a later layer: a border, its own row's or a
/// neighbouring group's outline, would otherwise strike its label through.
pub(super) fn drop_line(hint: Option<&super::link::DropHint>, below: bool) -> Vec<AnyElement> {
    let line = div()
        .absolute()
        .left_0()
        .right_0()
        .h(px(2.))
        .bg(theme::accent());
    let line = if below { line.bottom_0() } else { line.top_0() };
    let pill = hint.map(|hint| {
        let pill = match hint {
            super::link::DropHint::Link(chat) => drop_pill(
                "link.svg",
                crate::i18n::tf("format.link_to_chat_named", &[("chat", chat.clone())]),
            ),
            super::link::DropHint::Unlink(chat) => drop_pill(
                "link-2-off.svg",
                crate::i18n::tf("format.unlink_from_chat", &[("chat", chat.clone())]),
            ),
        }
        .absolute()
        .right(rems(6. / 16.));
        let pill = if below {
            pill.bottom(rems(-7. / 16.))
        } else {
            pill.top(rems(-7. / 16.))
        };
        // Above the hover-preview sidebar (1) that may hold this row, below menus (8).
        gpui::deferred(pill).with_priority(2).into_any_element()
    });
    std::iter::once(line.into_any_element())
        .chain(pill)
        .collect()
}

pub(super) fn marker_prefix(kind: DropKind) -> &'static str {
    match kind {
        DropKind::Pinned => "pinned",
        DropKind::Project => "project",
        DropKind::Leaf => "leaf",
    }
}

pub(crate) fn project_header_icon(collapsed: bool) -> ChatIcon {
    ChatIcon::generic(if collapsed {
        "folder.svg"
    } else {
        "folder-open.svg"
    })
}

impl Sidebar {
    pub(super) fn live_title(&self, session_id: &str, cx: &App) -> Option<String> {
        self.shell
            .upgrade()?
            .read(cx)
            .attached_title(session_id, cx)
    }

    /// A hovered row swaps its status for the "more" button in the same slot.
    /// A hover style cannot do this: gpui-pre resolves group hover differently
    /// in prepaint and paint, and a child hidden in one and shown in the other
    /// panics.
    pub(super) fn set_row_hovered(&mut self, id: &str, hovered: bool, cx: &mut Context<Self>) {
        if hovered {
            self.hovered_row = Some(id.to_owned());
        } else if self.hovered_row.as_deref() == Some(id) {
            self.hovered_row = None;
        } else {
            return;
        }
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_tab_row(
        &self,
        node: NodeRow,
        layout: PaneLayout,
        members: Vec<SessionDisplay>,
        attention: AttentionState,
        shortcut_index: Option<u8>,
        drop_kind: DropKind,
        parent_id: Option<String>,
        visible_ids: Vec<String>,
        group: GroupHead,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = self.tab_selected(&node.id, cx);
        // An expanded group's head drops after the whole group, so the group draws that line.
        let after_group = group.in_group && group.links.is_some();
        let live = members
            .iter()
            .any(|member| member.status == SessionStatus::Running);
        let pane_count = layout.root.leaves().len();
        let label = tab_label_live(&layout, &members, |id| self.live_title(id, cx));
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
                group,
                None,
                cx,
            )
        } else {
            let show_shortcut = self.show_shortcut_pills && shortcut_index.is_some();
            let hovered = self.hovered_row.as_deref() == Some(node.id.as_str());
            let trailing = if show_shortcut {
                sidebar_row_trailing_slot()
                    .children(shortcut_index.map(|index| tab_shortcut_pill(index, active)))
            } else if hovered {
                sidebar_row_trailing_slot().child(
                    IconButton::new(
                        SharedString::from(format!("sidebar-tab-actions-{}", node.id)),
                        "more-horizontal.svg",
                    )
                    .size(IconButtonSize::Xs)
                    .stop_click_propagation(true)
                    .tooltip(crate::i18n::text("ui.more_actions"))
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
                    }),
                )
            } else if let Some(attention) = group.attention {
                // A collapsed group shows its most urgent mission, not the chat's own status.
                sidebar_row_trailing_slot().child(group_state_glyph(&node.id, attention))
            } else {
                sidebar_row_trailing_slot().child(
                    self.render_rollup_attention(
                        self.tab_status_rollup(&members, cx),
                        Some(node.id.clone()),
                        members
                            .iter()
                            .any(|member| self.archiving_sessions.contains(&member.session_id)),
                        pane_count == 1,
                        SharedString::from(format!("attention-{}", node.id)),
                        cx,
                    ),
                )
            };
            let collapsed_head = group.summary.is_some();
            super::elements::sidebar_item_row_with(
                SharedString::from(format!("sidebar-tab-{}", node.id)),
                &SidebarItemView {
                    title: label.clone(),
                    icon: leaf_icon,
                    live,
                    selected: active,
                    pinned: node.pinned_position.is_some(),
                },
                group.summary,
                !group.in_group,
            )
            .when(group.in_group, |row| {
                super::link::head_shape(row, collapsed_head)
            })
            .when(self.front_page.is_remote(), |row| {
                row.debug_selector(|| "REMOTE_SIDEBAR_ROW".into())
            })
            .when(!live, |row| row.text_color(theme::faint()))
            .children(group.links.map(link_count))
            .child(trailing)
            .on_hover({
                let id = node.id.clone();
                cx.listener(move |this, hovered: &bool, _, cx| {
                    this.set_row_hovered(&id, *hovered, cx)
                })
            })
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
        let link_chats = self.tab_link_chats(&members, cx);
        self.decorate_draggable_row(
            base,
            &node,
            label,
            Some((leaf_icon, live)),
            drop_kind,
            parent_id,
            visible_ids,
            None,
            link_chats,
            after_group,
            cx,
        )
    }

    /// A tab's agent chats by session, each with its title.
    fn tab_link_chats(&self, members: &[SessionDisplay], cx: &App) -> Vec<(String, String)> {
        members
            .iter()
            .filter(|member| super::link::is_agent_chat(member))
            .map(|member| {
                (
                    member.session_id.clone(),
                    member.label(self.live_title(&member.session_id, cx).as_deref()),
                )
            })
            .collect()
    }

    /// The name a drop that links to a tab gives it: its one agent chat's
    /// title, or the tab's own when it holds several.
    pub(super) fn tab_link_label(
        &self,
        layout: &PaneLayout,
        members: &[SessionDisplay],
        cx: &App,
    ) -> String {
        let label = tab_label_live(layout, members, |id| self.live_title(id, cx));
        match self.tab_link_chats(members, cx).as_slice() {
            [(_, title)] => title.clone(),
            _ => label,
        }
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
        group: Option<(String, super::link::MissionShape)>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (group, shape) = group.unzip();
        let active = self.mission_selected(&summary.mission.id, cx);
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
                (ChatIcon::mission(), summary.any_session_live),
                attention,
                shortcut_index,
                active,
                GroupHead::default(),
                shape,
                cx,
            )
        } else {
            let show_shortcut = self.show_shortcut_pills && shortcut_index.is_some();
            let hovered = self.hovered_row.as_deref() == Some(node.id.as_str());
            let trailing = if show_shortcut {
                sidebar_row_trailing_slot()
                    .children(shortcut_index.map(|index| tab_shortcut_pill(index, active)))
            } else if hovered {
                sidebar_row_trailing_slot().child(
                    IconButton::new(
                        SharedString::from(format!(
                            "sidebar-mission-actions-{}",
                            summary.mission.id
                        )),
                        "more-horizontal.svg",
                    )
                    .size(IconButtonSize::Xs)
                    .stop_click_propagation(true)
                    .tooltip(crate::i18n::text("ui.more_actions"))
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
            } else {
                sidebar_row_trailing_slot()
                    .child(self.render_mission_attention(&summary, &node.id, cx))
            };
            super::elements::sidebar_item_row_with(
                SharedString::from(format!("sidebar-mission-{}", summary.mission.id)),
                &SidebarItemView {
                    title: label.clone(),
                    icon: ChatIcon::mission(),
                    live: summary.any_session_live,
                    selected: active,
                    pinned: node.pinned_position.is_some(),
                },
                None,
                shape.is_none(),
            )
            .when_some(shape, |row, shape| shape.apply(row))
            .when(self.front_page.is_remote(), |row| {
                row.debug_selector(|| "REMOTE_SIDEBAR_ROW".into())
            })
            .child(trailing)
            .on_hover({
                let id = node.id.clone();
                cx.listener(move |this, hovered: &bool, _, cx| {
                    this.set_row_hovered(&id, *hovered, cx)
                })
            })
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
        self.decorate_draggable_row(
            base,
            &node,
            label,
            None,
            drop_kind,
            parent_id,
            visible_ids,
            group,
            Vec::new(),
            false,
            cx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_project(
        &self,
        node: NodeRow,
        project: crate::surfaces::front_page::model::ProjectDisplay,
        nested: Vec<SidebarRow>,
        attention: AttentionState,
        visible_projects: Vec<String>,
        tab_index_by_node: &HashMap<String, u8>,
        launcher_row: Option<&ProjectScope>,
        rows: &[SidebarRow],
        groups: &super::link::LinkGroups,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let collapsed = self
            .settings(cx)
            .sidebar_collapsed_projects
            .contains(&project.id);
        let live = nested.iter().any(SidebarRow::is_live);
        let launcher_here =
            matches!(launcher_row, Some(ProjectScope::Project(id)) if *id == project.id);
        let selected = launcher_row.is_none()
            && self.active_project_id.as_deref() == Some(project.id.as_str());
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
                (project_header_icon(collapsed), live),
                if collapsed {
                    attention
                } else {
                    AttentionState::None
                },
                None,
                selected,
                GroupHead::default(),
                None,
                cx,
            )
        } else {
            sidebar_row_shell(
                SharedString::from(format!("sidebar-project-{}", project.id)),
                selected,
                false,
                true,
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
            .child(sidebar_icon(project_header_icon(collapsed), live))
            .child(
                Tooltip::new(
                    SharedString::from(format!("project-cwd-{}", project.id)),
                    project.cwd.clone(),
                    project_row_label(project.name.clone()).debug_selector({
                        let id = project.id.clone();
                        move || format!("SIDEBAR_PROJECT:{id}")
                    }),
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
                        crate::i18n::text("ui.new_in_project"),
                        move |_, window, cx| {
                            create_menu_root.update(cx, |this, cx| {
                                this.open_launcher_in(
                                    ProjectScope::Project(create_project_id.clone()),
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
                        crate::i18n::text("ui.project_actions"),
                        move |anchor, window, cx| {
                            menu_root.update(cx, |this, cx| {
                                this.open_project_menu(
                                    menu_project.clone(),
                                    MenuOrigin::Button(anchor),
                                    window,
                                    cx,
                                )
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
                    this.open_project_menu(
                        context_project.clone(),
                        MenuOrigin::Pointer(event.position),
                        window,
                        cx,
                    );
                }),
            )
            .into_any_element()
        };
        let label = project.name.clone();
        let project_id = project.id.clone();
        let nested_ids = nested
            .iter()
            .map(|row| row.node().id.clone())
            .filter(|id| groups.head(id).is_none())
            .collect::<Vec<_>>();
        let has_nested = !nested_ids.is_empty();
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
            let drag = SidebarNodeDrag::new(drag_node_id, drag_label, None);
            header_wrap = header_wrap
                .child(drag.measure())
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
                                None,
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
                .on_drop(cx.listener(|this, drag: &SidebarNodeDrag, window, cx| {
                    this.commit_sidebar_drop(&drag.node_id, window, cx);
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
                project_children()
                    .children(launcher_here.then(|| self.launcher_row(cx)))
                    .children(if nested.is_empty() {
                        if self.dragged_id.is_some() {
                            vec![self.render_empty_project_drop_area(node.id.clone(), cx)]
                        } else if launcher_here {
                            Vec::new()
                        } else {
                            vec![empty_sidebar_label(crate::i18n::text("ui.empty"))]
                        }
                    } else {
                        self.render_scope(
                            nested,
                            rows,
                            groups,
                            Some(project_id.clone()),
                            DropKind::Leaf,
                            Some(node.id.clone()),
                            tab_index_by_node,
                            cx,
                        )
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
            .on_drop(cx.listener(|this, drag: &SidebarNodeDrag, window, cx| {
                this.commit_sidebar_drop(&drag.node_id, window, cx);
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
        let marker_prefix = marker_prefix(kind);
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
                        None,
                        cx,
                    );
                },
            ))
            .on_drop(cx.listener(|this, drag: &SidebarNodeDrag, window, cx| {
                this.commit_sidebar_drop(&drag.node_id, window, cx);
            }));
        if active {
            divider = match &self.drop_hint {
                Some(hint) => divider.children(drop_line(Some(hint), false)),
                None => divider.border_t_2().border_color(theme::accent()),
            };
        }
        divider.into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn decorate_draggable_row(
        &self,
        row: AnyElement,
        node: &NodeRow,
        label: String,
        icon: Option<(ChatIcon, bool)>,
        kind: DropKind,
        parent_id: Option<String>,
        visible_ids: Vec<String>,
        group: Option<String>,
        link_chats: Vec<(String, String)>,
        after_group: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.front_page.supports(&["move"])
            && !self.front_page.supports(&["reorderPinned"])
            && !self.front_page.supports(&["linkMission"])
        {
            return row;
        }
        let drag = SidebarNodeDrag::new(node.id.clone(), label.clone(), icon);
        let drag_root = cx.entity();
        let hovered_id = node.id.clone();
        let marker_prefix = marker_prefix(kind);
        let marker_before = format!("{marker_prefix}:{}:false", node.id);
        let marker_after = format!("{marker_prefix}:{}:true", node.id);
        let active_before = self.drop_marker.as_deref() == Some(marker_before.as_str());
        let active_after =
            !after_group && self.drop_marker.as_deref() == Some(marker_after.as_str());
        let link_active = self.drop_marker.as_deref() == Some(format!("link:{}", node.id).as_str());
        let link_label = match link_chats.as_slice() {
            [(_, title)] => title.clone(),
            _ => label,
        };
        let dragged_here = node.node_type == NodeType::Mission
            && self.dragged_id.as_deref() == Some(node.id.as_str());
        let mut wrapper = crate::platform_ui::sidebar_row_wrapper(SharedString::from(format!(
            "sidebar-drag-{}",
            node.id
        )))
        .relative()
        .cursor_move()
        .when(dragged_here, |wrapper| wrapper.opacity(0.45))
        .child(row)
        .child(drag.measure())
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
                let mut after = event.event.position.y > event.bounds.center().y;
                if !link_chats.is_empty()
                    && this.can_link_drop(&dragged, &hovered_id, &link_chats, cx)
                {
                    match super::link::chat_row_zone(
                        f32::from(event.event.position.y),
                        f32::from(event.bounds.top()),
                        f32::from(event.bounds.size.height),
                    ) {
                        super::link::ChatRowZone::Link => {
                            this.set_link_drop(
                                super::link::LinkDrop {
                                    mission_node: dragged,
                                    tab_node: hovered_id.clone(),
                                    chats: link_chats.clone(),
                                },
                                cx,
                            );
                            return;
                        }
                        zone => after = zone == super::link::ChatRowZone::After,
                    }
                }
                this.update_row_drop_target(
                    &dragged,
                    kind,
                    parent_id.as_deref(),
                    &visible_ids,
                    &hovered_id,
                    after,
                    format!("{marker_prefix}:{hovered_id}:{after}"),
                    group.clone(),
                    cx,
                );
            },
        ))
        .on_drop(cx.listener(|this, drag: &SidebarNodeDrag, window, cx| {
            this.commit_sidebar_drop(&drag.node_id, window, cx);
        }));
        if link_active {
            wrapper = wrapper
                .border_1()
                .border_color(theme::accent())
                .rounded_sm()
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_end()
                        .pr(rems(6. / 16.))
                        .child(drop_pill(
                            "link.svg",
                            crate::i18n::tf("format.link_to_chat_named", &[("chat", link_label)]),
                        )),
                );
        } else if active_before || active_after {
            wrapper = match &self.drop_hint {
                Some(hint) => wrapper.children(drop_line(Some(hint), active_after)),
                None if active_before => wrapper.border_t_2().border_color(theme::accent()),
                None => wrapper.border_b_2().border_color(theme::accent()),
            };
        }
        wrapper.into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_inline_rename_row(
        &self,
        kind: NodeType,
        id: &str,
        disclosure_icon: Option<&'static str>,
        icon: (ChatIcon, bool),
        attention: AttentionState,
        shortcut_index: Option<u8>,
        shortcut_selected: bool,
        group: GroupHead,
        shape: Option<super::link::MissionShape>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(rename) = self
            .rename
            .as_ref()
            .filter(|rename| rename.target.matches(kind, id))
        else {
            return div().into_any_element();
        };
        let (icon, live) = icon;
        let input = rename.input.clone();
        let row_id = SharedString::from(format!("sidebar-rename-{id}"));
        let collapsed_head = group.summary.is_some();
        let field = div().min_w(px(0.)).flex_1().child(input);
        let bordered = !group.in_group && shape.is_none();
        sidebar_row_shell(row_id.clone(), true, false, bordered)
            .when(group.in_group, |row| {
                super::link::head_shape(row, collapsed_head)
            })
            .when_some(shape, |row, shape| shape.apply(row))
            .children(disclosure_icon.map(|icon| {
                svg()
                    .path(icon)
                    .size(rems(12. / 16.))
                    .flex_none()
                    .text_color(theme::text())
            }))
            .child(super::elements::sidebar_mark(
                row_id,
                sidebar_icon(icon, live),
            ))
            .map(|row| match group.summary {
                Some(summary) => row.child(field.flex().flex_col().child(summary)),
                None => row.child(field),
            })
            .children(group.links.map(link_count))
            .child(match group.attention {
                Some(attention) => group_state_glyph(id, attention).into_any_element(),
                None => attention_indicator(attention).into_any_element(),
            })
            .children(
                shortcut_index
                    .filter(|_| self.show_shortcut_pills)
                    .map(|index| {
                        sidebar_row_trailing_slot()
                            .child(tab_shortcut_pill(index, shortcut_selected))
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
