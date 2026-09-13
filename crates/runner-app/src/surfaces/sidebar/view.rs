use super::elements::attention_indicator;
use super::elements::empty_sidebar_label;
use super::elements::section_title;
use super::elements::workspace_new_chat_row;
use super::elements::workspace_row;

use super::*;
use crate::surfaces::sidebar_logic::{
    attention_rollups, indicator_visible, rollup_attention_state, take_drag_state, AttentionState,
    DropKind,
};
use crate::*;
use gpui::{svg, FontWeight};
use runner_backend::repo::node::NodeType;

// Shared with the layout probe so the production flex constraints stay under test.
pub(super) fn sidebar_scroll_frame() -> gpui::Div {
    div().relative().min_h(px(0.)).flex_1().flex().flex_col()
}

pub(super) fn sidebar_scroll_container(
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

impl Sidebar {
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
        let rows = self.resolved_sidebar_rows(cx);
        let mut pinned = rows
            .iter()
            .filter(|row| row.node().pinned_position.is_some())
            .cloned()
            .collect::<Vec<_>>();
        pinned.sort_by_key(|row| row.node().pinned_position);
        let has_pinned = !pinned.is_empty();
        let root_rows = self.scope_rows(&rows, None);
        let project_nodes = self
            .app_store
            .read(cx)
            .nodes
            .iter()
            .filter(|node| {
                node.parent_id.is_none()
                    && node.pinned_position.is_none()
                    && node.node_type == NodeType::Project
            })
            .cloned()
            .collect::<Vec<_>>();
        let tab_index_by_node = self.tab_index_by_node.clone();
        let rollups = attention_rollups(
            rows.iter()
                .filter(|row| row.node().pinned_position.is_none())
                .map(|row| (row.node().parent_id.clone(), row.attention())),
        );
        let project_attention = rollup_attention_state(project_nodes.iter().map(|project| {
            rollups
                .get(&Some(project.id.clone()))
                .copied()
                .unwrap_or_default()
        }));
        let root_attention = rollups.get(&None).copied().unwrap_or_default();

        let mut scroll = sidebar_scroll_container("sidebar-node-scroll", &self.scroll).on_drop(
            cx.listener(|this, drag: &SidebarNodeDrag, _, cx| {
                this.commit_sidebar_drop(&drag.node_id, cx);
            }),
        );
        if !pinned.is_empty() {
            let visible = pinned
                .iter()
                .map(|row| row.node().id.clone())
                .collect::<Vec<_>>();
            scroll = scroll.child(
                div()
                    .flex()
                    .flex_col()
                    .child(section_title("PINNED"))
                    .child(
                        div()
                            .px_3()
                            .pt_1()
                            .flex()
                            .flex_col()
                            .gap(rems(2. / 16.))
                            .children(pinned.into_iter().map(|row| {
                                let project_id =
                                    node_project_id(&self.app_store.read(cx).nodes, row.node());
                                let shortcut_index = tab_index_by_node.get(&row.node().id).copied();
                                self.render_sidebar_row(
                                    row,
                                    project_id,
                                    shortcut_index,
                                    DropKind::Pinned,
                                    None,
                                    visible.clone(),
                                    cx,
                                )
                            }))
                            .children(self.dragged_id.is_some().then(|| {
                                self.render_end_drop_divider(DropKind::Pinned, None, visible, cx)
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
                    "PROJECTS",
                    projects_open,
                    (!projects_open).then_some(project_attention),
                    "Add project",
                    move |this, cx| {
                        let open = !this.settings(cx).sidebar_projects_open;
                        this.update_app_settings(cx, true, |settings| {
                            settings.sidebar_projects_open = open;
                            true
                        });
                        cx.notify();
                    },
                    move |window, cx| {
                        project_add_root.update(cx, |this, cx| this.open_project_modal(window, cx));
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
                            vec![empty_sidebar_label("No projects yet.")]
                        } else {
                            project_nodes
                                .into_iter()
                                .filter_map(|node| {
                                    let project = self
                                        .app_store
                                        .read(cx)
                                        .projects
                                        .iter()
                                        .find(|project| {
                                            node.ref_id.as_deref() == Some(project.id.as_str())
                                        })?
                                        .clone();
                                    let nested = self.scope_rows(&rows, Some(&node.id));
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
        let create_menu = self.create_menu.clone();
        scroll = scroll.child(
            div()
                .mt_5()
                .flex_1()
                .flex()
                .flex_col()
                .child(
                    div()
                        .px_5()
                        .pb(rems(6. / 16.))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .id("toggle-chats-section")
                                .group("sidebar-chats-section")
                                .min_w(px(0.))
                                .flex()
                                .items_center()
                                .gap(rems(6. / 16.))
                                .cursor_pointer()
                                .text_color(theme::faint())
                                .hover(|header| header.text_color(theme::muted()))
                                .child(
                                    div()
                                        .min_w(px(0.))
                                        .text_size(theme::text_caption())
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("RECENTS"),
                                )
                                .child(
                                    svg()
                                        .flex_none()
                                        .path(if chats_open {
                                            "chevron-down.svg"
                                        } else {
                                            "chevron-right.svg"
                                        })
                                        .size(rems(10. / 16.))
                                        .text_color(theme::faint())
                                        .group_hover("sidebar-chats-section", |icon| {
                                            icon.text_color(theme::muted())
                                        }),
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let open = !this.settings(cx).sidebar_chats_open;
                                    this.update_app_settings(cx, true, |settings| {
                                        settings.sidebar_chats_open = open;
                                        true
                                    });
                                    cx.notify();
                                })),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(rems(6. / 16.))
                                .children(
                                    (!chats_open).then(|| attention_indicator(root_attention)),
                                )
                                .child(create_menu),
                        ),
                )
                .children(chats_open.then(|| {
                    let visible = root_rows
                        .iter()
                        .map(|row| row.node().id.clone())
                        .collect::<Vec<_>>();
                    let has_rows = !root_rows.is_empty();
                    div()
                        .id("root-chat-scope")
                        .flex_1()
                        .px_3()
                        .pt_1()
                        .flex()
                        .flex_col()
                        .gap(rems(2. / 16.))
                        .children(if root_rows.is_empty() {
                            vec![empty_sidebar_label("No chats yet.")]
                        } else {
                            root_rows
                                .into_iter()
                                .map(|row| {
                                    let shortcut_index =
                                        tab_index_by_node.get(&row.node().id).copied();
                                    self.render_sidebar_row(
                                        row,
                                        None,
                                        shortcut_index,
                                        DropKind::Leaf,
                                        None,
                                        visible.clone(),
                                        cx,
                                    )
                                })
                                .collect()
                        })
                        .children((has_rows && self.dragged_id.is_some()).then(|| {
                            self.render_end_drop_divider(DropKind::Leaf, None, visible, cx)
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.open_sidebar_context_menu(
                                    event.position,
                                    160.,
                                    vec![(
                                        UiMenuItem::new("New chat").icon("message-square-plus.svg"),
                                        SidebarMenuAction::NewChat(None),
                                    )],
                                    window,
                                    cx,
                                );
                            }),
                        )
                })),
        );
        let workspace_rows = WORKSPACE_ENTRIES.map(|entry| {
            let active = entry.selected(&route);
            match entry {
                WorkspaceEntry::NewTab => {
                    let shell = self.shell.clone();
                    let shortcut = keymap::effective_binding(
                        "new-chat",
                        &self.app_store.read(cx).settings.keymap_overrides,
                    )
                    .map(|combo| keymap::format_combo(&combo));
                    workspace_new_chat_row(shortcut, move |window, cx| {
                        if let Some(shell) = shell.upgrade() {
                            shell.update(cx, |shell, shell_cx| {
                                let project_id = shell.active_project_id(shell_cx);
                                shell.open_sidebar_chat_modal(
                                    project_id.as_deref(),
                                    window,
                                    shell_cx,
                                )
                            });
                        }
                    })
                }
                WorkspaceEntry::Runner => {
                    workspace_row("workspace-runner", "terminal.svg", "runner", active, {
                        let shell = self.shell.clone();
                        move |window, cx| {
                            if let Some(shell) = shell.upgrade() {
                                shell.update(cx, |shell, shell_cx| {
                                    shell.open_runners(window, shell_cx)
                                });
                            }
                        }
                    })
                }
                WorkspaceEntry::Crew => {
                    workspace_row("workspace-crew", "users.svg", "crew", active, {
                        let shell = self.shell.clone();
                        move |window, cx| {
                            if let Some(shell) = shell.upgrade() {
                                shell.update(cx, |shell, shell_cx| {
                                    shell.open_crews(window, shell_cx)
                                });
                            }
                        }
                    })
                }
            }
        });
        div()
            .min_h(px(0.))
            .flex_1()
            .flex()
            .flex_col()
            .overflow_hidden()
            .pb_3()
            .child(
                crate::platform_ui::sidebar_section()
                    .child(section_title("WORKSPACE"))
                    .child(
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
                    .bg(theme::sidebar_selected_border()),
            )
            .child(
                sidebar_scroll_frame()
                    .child(scroll)
                    .child(self.scrollbar.clone()),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_section_header<F, G>(
        &self,
        label: &'static str,
        open: bool,
        attention: Option<AttentionState>,
        plus_title: &'static str,
        on_toggle: F,
        on_plus: G,
        cx: &mut Context<Self>,
    ) -> AnyElement
    where
        F: Fn(&mut Sidebar, &mut Context<Sidebar>) + 'static,
        G: Fn(&mut Window, &mut App) + 'static,
    {
        div()
            .px_5()
            .pb(rems(6. / 16.))
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                div()
                    .id(SharedString::from(format!("toggle-{label}")))
                    .group("sidebar-section-toggle")
                    .min_w(px(0.))
                    .flex()
                    .items_center()
                    .gap(rems(6. / 16.))
                    .cursor_pointer()
                    .text_color(theme::faint())
                    .hover(|header| header.text_color(theme::muted()))
                    .child(
                        div()
                            .text_size(theme::text_caption())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(label),
                    )
                    .child(
                        svg()
                            .flex_none()
                            .path(if open {
                                "chevron-down.svg"
                            } else {
                                "chevron-right.svg"
                            })
                            .size(rems(10. / 16.))
                            .text_color(theme::faint())
                            .group_hover("sidebar-section-toggle", |icon| {
                                icon.text_color(theme::muted())
                            }),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| on_toggle(this, cx))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(6. / 16.))
                    .children(attention.map(attention_indicator))
                    .child(
                        IconButton::new(SharedString::from(format!("add-{label}")), "plus.svg")
                            .size(IconButtonSize::Sm)
                            .tooltip(plus_title)
                            .on_press(on_plus),
                    ),
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
                cx,
            ),
        }
    }
}
