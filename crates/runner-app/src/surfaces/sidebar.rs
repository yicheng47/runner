//! Native sidebar: durable node tree, project containers, tab rows, and attention.

use std::path::Path;

use super::*;
use crate::surfaces::sidebar_logic::{
    attention_rollups, complete_unpinned_scope_order, container_drop_target, list_drop_target,
    mission_attention_state, ordered_pinned_node_ids_after_drop,
    ordered_root_node_ids_after_project_drop, ordered_visible_node_ids_after_drop,
    rollup_attention_state, tab_attention_state, AttentionState, DropKind, DropTarget,
};
use crate::*;
use gpui::{
    radians, svg, DragMoveEvent, FontWeight, PathPromptOptions, Transformation, WeakEntity,
};
use runner_app::ui::{
    working_dir_text_field, ButtonVariant, ConfirmDialog, Field, Modal, OverlayWidth, TextField,
    Tooltip, WorkingDirField,
};
use runner_backend::ops::mission::{MissionActivityState, MissionSummary};
use runner_backend::repo::node::{NodeRow, NodeType};

const SIDEBAR_ROW_FONT_SIZE: f32 = 13.;

// Shared with the layout probe so the production flex constraints stay under test.
fn sidebar_scroll_frame() -> gpui::Div {
    div().relative().min_h(px(0.)).flex_1().flex().flex_col()
}

fn sidebar_scroll_container(
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

#[derive(Clone, Copy)]
enum ArchiveErrorTarget {
    App,
    Chat,
}

#[derive(Clone)]
enum SidebarRow {
    Tab {
        node: NodeRow,
        layout: PaneLayout,
        members: Vec<DirectSessionEntry>,
        attention: AttentionState,
    },
    Mission {
        node: NodeRow,
        summary: MissionSummary,
        attention: AttentionState,
    },
}

impl SidebarRow {
    fn node(&self) -> &NodeRow {
        match self {
            Self::Tab { node, .. } | Self::Mission { node, .. } => node,
        }
    }

    fn attention(&self) -> AttentionState {
        match self {
            Self::Tab { attention, .. } | Self::Mission { attention, .. } => *attention,
        }
    }

    fn is_live(&self) -> bool {
        match self {
            Self::Tab { members, .. } => members
                .iter()
                .any(|member| member.status == SessionStatus::Running),
            Self::Mission { summary, .. } => summary.any_session_live,
        }
    }
}

#[derive(Clone)]
enum SidebarRenameTarget {
    Tab {
        node_id: String,
        original: String,
    },
    Project {
        project_id: String,
        original: String,
    },
    Mission {
        mission_id: String,
        original: String,
    },
}

impl SidebarRenameTarget {
    fn matches(&self, kind: NodeType, id: &str) -> bool {
        match (self, kind) {
            (Self::Tab { node_id, .. }, NodeType::Tab) => node_id == id,
            (Self::Project { project_id, .. }, NodeType::Project) => project_id == id,
            (Self::Mission { mission_id, .. }, NodeType::Mission) => mission_id == id,
            _ => false,
        }
    }
}

pub(crate) struct SidebarRename {
    target: SidebarRenameTarget,
    input: Entity<TextField>,
}

pub(crate) struct ProjectModal {
    cwd: Entity<TextField>,
    name: Entity<TextField>,
    browse_focus: FocusHandle,
    close_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
    error: Option<String>,
    submitting: bool,
}

#[derive(Clone)]
struct SidebarNodeDrag {
    node_id: String,
    label: String,
}

impl Render for SidebarNodeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(rems(220. / 16.))
            .px_3()
            .py_2()
            .rounded_sm()
            .border_1()
            .border_color(theme::sidebar_selected_border())
            .bg(theme::sidebar_selected())
            .shadow_lg()
            .text_size(rems(SIDEBAR_ROW_FONT_SIZE / 16.))
            .text_color(theme::text())
            .child(self.label.clone())
    }
}

#[derive(Clone)]
enum SidebarMenuAction {
    NewChat(Option<String>),
    NewMission(Option<String>),
    OpenInNewWindow(String),
    TogglePin { node_id: String, pinned: bool },
    Rename(SidebarRenameTarget),
    RemoveTabFromProject(Vec<String>),
    RemoveMissionFromProject(String),
    ArchiveTab(Vec<String>),
    ArchiveMission(String),
    DeleteProject(String),
}

pub(crate) struct Sidebar {
    shell: WeakEntity<NativeRoot>,
    app_store: Entity<AppStore>,
    store_revisions: StoreRevisions,
    scroll: ScrollHandle,
    scrollbar: Entity<Scrollbar>,
    create_menu: Entity<PopoverMenu>,
    context_menu: Option<Entity<ContextMenu>>,
    rename: Option<SidebarRename>,
    archiving_sessions: HashSet<String>,
    archiving_missions: HashSet<String>,
    active_project_id: Option<String>,
    dragged_id: Option<String>,
    drop_target: Option<DropTarget>,
    drop_marker: Option<String>,
    _rename_focus_subscription: Option<Subscription>,
    _store_subscription: Subscription,
}

impl Sidebar {
    pub(crate) fn new(
        shell: WeakEntity<NativeRoot>,
        app_store: Entity<AppStore>,
        active_project_id: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let scroll = ScrollHandle::new();
        let scroll_owner = cx.entity_id();
        let scrollbar = cx.new(|_| Scrollbar::app(scroll.clone(), scroll_owner));
        let root = cx.entity();
        let create_menu = cx.new(move |menu_cx| {
            let action_root = root.clone();
            PopoverMenu::new(
                "sidebar-create",
                menu_cx.focus_handle(),
                vec![
                    UiMenuItem::new("New chat").icon("message-square-plus.svg"),
                    UiMenuItem::new("New mission").icon("flag.svg"),
                ],
                Rc::new(move |index, window, cx| {
                    action_root.update(cx, |this, cx| {
                        this.handle_sidebar_create_action(index, window, cx);
                    });
                }),
                menu_cx,
            )
            .min_width(px(160.))
            .trigger_size(IconButtonSize::Sm)
            .trigger_icon("plus.svg")
            .without_trigger_tooltip()
        });
        let store_revisions = app_store.read(cx).revisions;
        Self {
            shell,
            app_store: app_store.clone(),
            store_revisions,
            scroll,
            scrollbar,
            create_menu,
            context_menu: None,
            rename: None,
            archiving_sessions: HashSet::new(),
            archiving_missions: HashSet::new(),
            active_project_id,
            dragged_id: None,
            drop_target: None,
            drop_marker: None,
            _rename_focus_subscription: None,
            _store_subscription: cx.observe(&app_store, |this, _, cx| {
                this.handle_store_update(cx);
            }),
        }
    }

    fn core<'a>(&self, cx: &'a App) -> &'a AppCore {
        &self.app_store.read(cx).core
    }

    fn settings<'a>(&self, cx: &'a App) -> &'a AppSettings {
        &self.app_store.read(cx).settings
    }

    fn update_app_settings(
        &self,
        cx: &mut Context<Self>,
        persist: bool,
        update: impl FnOnce(&mut AppSettings) -> bool,
    ) -> bool {
        self.app_store.update(cx, |store, store_cx| {
            store.update_settings(update, persist, store_cx)
        })
    }

    fn refresh_store(&self, refresh: StoreRefreshKind, cx: &mut Context<Self>) {
        self.app_store
            .update(cx, |store, store_cx| store.refresh(refresh, store_cx));
    }

    fn handle_store_update(&mut self, cx: &mut Context<Self>) {
        let revisions = self.app_store.read(cx).revisions;
        let previous = self.store_revisions;
        self.store_revisions = revisions;
        if revisions.nodes != previous.nodes
            || revisions.projects != previous.projects
            || revisions.missions != previous.missions
            || revisions.sessions != previous.sessions
            || revisions.activity != previous.activity
            || revisions.settings != previous.settings
        {
            cx.notify();
        }
    }

    fn report_error(&self, error: String, cx: &mut Context<Self>) {
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        cx.defer(move |cx| {
            shell.update(cx, |shell, shell_cx| {
                shell.error = Some(error);
                shell_cx.notify();
            });
        });
    }

    fn schedule_shell_notify(&self, cx: &mut Context<Self>) {
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        cx.defer(move |cx| {
            shell.update(cx, |_, shell_cx| shell_cx.notify());
        });
    }

    fn focus_shell_terminal(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        window.defer(cx, move |window, cx| {
            shell.update(cx, |shell, shell_cx| {
                shell.focus_active_terminal(window, shell_cx);
            });
        });
    }
}

impl NativeRoot {
    pub(crate) fn tab_label(&self, layout: &PaneLayout, cx: &App) -> String {
        if let Some(name) = &layout.name {
            return name.clone();
        }
        let labels = layout
            .session_ids()
            .into_iter()
            .filter_map(|session_id| self.session_entry(&session_id, cx))
            .map(session_label)
            .collect::<Vec<_>>();
        if labels.is_empty() {
            "Empty tab".into()
        } else {
            labels.join(" + ")
        }
    }

    pub(crate) fn prune_sidebar_collapse_state(&mut self, cx: &mut Context<Self>) {
        let project_ids = self
            .app_store
            .read(cx)
            .projects
            .iter()
            .map(|project| project.id.clone())
            .collect::<std::collections::HashSet<_>>();
        self.app_store.update(cx, |store, store_cx| {
            store.update_settings(
                |settings| {
                    let previous = settings.sidebar_collapsed_projects.len();
                    settings
                        .sidebar_collapsed_projects
                        .retain(|id| project_ids.contains(id));
                    previous != settings.sidebar_collapsed_projects.len()
                },
                true,
                store_cx,
            );
        });
    }

    pub(crate) fn prune_store_dependent_window_state(&mut self, cx: &mut Context<Self>) {
        let sessions = &self.app_store.read(cx).sessions;
        let visible = sessions
            .iter()
            .map(|session| session.session_id.clone())
            .collect::<std::collections::HashSet<_>>();
        self.attached.retain(|id, _| visible.contains(id));
        let direct_visible = sessions
            .iter()
            .map(|session| session.session_id.as_str())
            .collect::<std::collections::HashSet<_>>();
        self.pane_action_menus
            .retain(|id, _| direct_visible.contains(id.as_str()));
        self.session_exit_codes
            .retain(|id, _| direct_visible.contains(id.as_str()));
    }

    pub(crate) fn dismiss_sidebar_transients(&mut self, cx: &mut Context<Self>) {
        self.sidebar.update(cx, |sidebar, sidebar_cx| {
            sidebar.dismiss_transients(sidebar_cx)
        });
        self.project_modal = None;
        self._project_cwd_subscription = None;
        self.project_delete_confirm = None;
        self.project_delete_busy = false;
        cx.notify();
    }

    pub(crate) fn sync_sidebar_window_activation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_window_activation(window, cx);
    }

    pub(crate) fn mark_active_tab_viewed(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(layout) = self.tabs.active() else {
            self.report_current_subjects(cx);
            return;
        };
        let tab_id = layout.id.clone();
        let member_ids = layout.session_ids();
        if !window.is_window_active() {
            self.report_current_subjects(cx);
            runner_backend::ops::window::mark_blurred(self.core(cx), &self.window_label);
            return;
        }
        match runner_backend::ops::node::node_mark_viewed(
            self.core(cx),
            &self.window_label,
            &tab_id,
            member_ids,
        ) {
            Ok(updated) => {
                self.app_store
                    .update(cx, |store, store_cx| store.replace_node(updated, store_cx));
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    pub(crate) fn sync_active_project_from_active_tab(&mut self, cx: &mut Context<Self>) {
        let active_project_id = self.tabs.active_tab_id().and_then(|tab_id| {
            let node = self
                .app_store
                .read(cx)
                .nodes
                .iter()
                .find(|node| node.id == tab_id)?;
            node_project_id(&self.app_store.read(cx).nodes, node)
        });
        self.sidebar.update(cx, |sidebar, sidebar_cx| {
            sidebar.set_active_project(active_project_id, sidebar_cx)
        });
    }

    pub(crate) fn active_project_id(&self, cx: &App) -> Option<String> {
        self.sidebar.read(cx).active_project_id.clone()
    }

    pub(crate) fn sidebar_archiving_session(&self, session_id: &str, cx: &App) -> bool {
        self.sidebar
            .read(cx)
            .archiving_sessions
            .contains(session_id)
    }

    pub(crate) fn set_sidebar_mission_archiving(
        &mut self,
        mission_id: String,
        archiving: bool,
        cx: &mut Context<Self>,
    ) {
        self.sidebar.update(cx, |sidebar, sidebar_cx| {
            sidebar.set_mission_archiving(mission_id, archiving, sidebar_cx)
        });
    }

    pub(crate) fn clear_sidebar_drag(&mut self, cx: &mut Context<Self>) {
        self.sidebar.update(cx, |sidebar, sidebar_cx| {
            sidebar.clear_sidebar_drag(sidebar_cx)
        });
    }

    pub(crate) fn archive_chat_sessions(
        &mut self,
        session_ids: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = self.active_focused_session_id();
        self.sidebar.update(cx, |sidebar, sidebar_cx| {
            sidebar.archive_sessions(
                session_ids,
                ArchiveErrorTarget::Chat,
                active,
                window,
                sidebar_cx,
            )
        });
    }

    fn activate_sidebar_session(
        &mut self,
        tab_id: &str,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.tabs.activate(tab_id) {
            return;
        }
        if let Some(layout) = self.tabs.active_mut() {
            layout.focus_session(session_id);
        }
        self.set_route(AppRoute::Chat, cx);
        self.activate_tab(tab_id, window, cx);
    }

    pub(crate) fn open_chat_session(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let tab_id = self
            .tabs
            .tabs()
            .iter()
            .find(|layout| layout.session_ids().iter().any(|id| id == session_id))
            .map(|layout| layout.id.clone());
        let Some(tab_id) = tab_id else {
            return false;
        };
        self.activate_sidebar_session(&tab_id, session_id, window, cx);
        true
    }
}

impl Sidebar {
    fn dismiss_transients(&mut self, cx: &mut Context<Self>) {
        self.create_menu
            .update(cx, |menu, menu_cx| menu.close(menu_cx));
        let had_context_menu = self.context_menu.take().is_some();
        self.rename = None;
        self._rename_focus_subscription = None;
        self.clear_sidebar_drag(cx);
        if had_context_menu {
            self.schedule_shell_notify(cx);
        }
        cx.notify();
    }

    fn set_active_project(&mut self, project_id: Option<String>, cx: &mut Context<Self>) {
        self.active_project_id = project_id;
        cx.notify();
    }

    fn set_mission_archiving(
        &mut self,
        mission_id: String,
        archiving: bool,
        cx: &mut Context<Self>,
    ) {
        let changed = if archiving {
            self.archiving_missions.insert(mission_id)
        } else {
            self.archiving_missions.remove(&mission_id)
        };
        if changed {
            cx.notify();
        }
    }

    fn open_project_modal(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        window.defer(cx, move |window, cx| {
            shell.update(cx, |shell, shell_cx| {
                shell.open_project_modal(window, shell_cx)
            });
        });
    }

    fn open_mission(&self, mission_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        window.defer(cx, move |window, cx| {
            shell.update(cx, |shell, shell_cx| {
                shell.open_mission(mission_id, window, shell_cx)
            });
        });
    }

    fn resolved_sidebar_rows(&self, cx: &App) -> Vec<SidebarRow> {
        let Some(shell) = self.shell.upgrade() else {
            return Vec::new();
        };
        let shell = shell.read(cx);
        let layouts = shell
            .tabs
            .tabs()
            .iter()
            .map(|layout| (layout.id.as_str(), layout))
            .collect::<HashMap<_, _>>();
        let sessions = self
            .app_store
            .read(cx)
            .sessions
            .iter()
            .map(|session| (session.session_id.as_str(), session))
            .collect::<HashMap<_, _>>();
        let missions = self
            .app_store
            .read(cx)
            .missions
            .iter()
            .map(|summary| (summary.mission.id.as_str(), summary))
            .collect::<HashMap<_, _>>();
        self.app_store
            .read(cx)
            .nodes
            .iter()
            .filter_map(|node| match node.node_type {
                NodeType::Tab => {
                    let layout = layouts.get(node.id.as_str())?;
                    let members = layout
                        .session_ids()
                        .into_iter()
                        .filter_map(|id| sessions.get(id.as_str()).map(|row| (*row).clone()))
                        .collect::<Vec<_>>();
                    if members.is_empty() {
                        return None;
                    }
                    let working = members.iter().any(|member| {
                        self.archiving_sessions.contains(&member.session_id)
                            || (member.status == SessionStatus::Running
                                && self
                                    .app_store
                                    .read(cx)
                                    .session_activity
                                    .get(&member.session_id)
                                    == Some(&SessionActivityState::Busy))
                    });
                    Some(SidebarRow::Tab {
                        node: node.clone(),
                        layout: (*layout).clone(),
                        members,
                        attention: tab_attention_state(
                            working,
                            node.last_completed_at.as_deref(),
                            node.last_viewed_at.as_deref(),
                        ),
                    })
                }
                NodeType::Mission => {
                    let summary = missions.get(node.ref_id.as_deref()?)?;
                    let idle = summary.activity == Some(MissionActivityState::Idle);
                    let attention = if self.archiving_missions.contains(&summary.mission.id) {
                        AttentionState::Working
                    } else {
                        mission_attention_state(summary.any_session_live, idle)
                    };
                    Some(SidebarRow::Mission {
                        node: node.clone(),
                        summary: (*summary).clone(),
                        attention,
                    })
                }
                NodeType::Project => None,
            })
            .collect()
    }

    fn scope_rows(&self, rows: &[SidebarRow], parent_id: Option<&str>) -> Vec<SidebarRow> {
        rows.iter()
            .filter(|row| {
                row.node().pinned_position.is_none() && row.node().parent_id.as_deref() == parent_id
            })
            .cloned()
            .collect()
    }

    fn activate_sidebar_session(
        &mut self,
        tab_id: &str,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        let tab_id = tab_id.to_owned();
        let session_id = session_id.to_owned();
        window.defer(cx, move |window, cx| {
            shell.update(cx, |shell, shell_cx| {
                shell.activate_sidebar_session(&tab_id, &session_id, window, shell_cx)
            });
        });
    }

    fn toggle_project(&mut self, project_id: &str, cx: &mut Context<Self>) {
        self.active_project_id = Some(project_id.to_owned());
        let project_id = project_id.to_owned();
        self.update_app_settings(cx, true, move |settings| {
            if !settings.sidebar_collapsed_projects.remove(&project_id) {
                settings.sidebar_collapsed_projects.insert(project_id);
            }
            true
        });
        cx.notify();
    }

    pub(crate) fn handle_sidebar_create_action(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match index {
            0 | 1 => {
                let Some(shell) = self.shell.upgrade() else {
                    return;
                };
                window.defer(cx, move |window, cx| {
                    shell.update(cx, |shell, shell_cx| match index {
                        0 => shell.open_sidebar_chat_modal(None, window, shell_cx),
                        1 => shell.open_start_mission_modal(None, None, window, shell_cx),
                        _ => unreachable!(),
                    });
                });
            }
            _ => unreachable!("sidebar create menu index"),
        }
    }

    fn begin_sidebar_rename(
        &mut self,
        target: SidebarRenameTarget,
        value: String,
        placeholder: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), value, placeholder, false)
                .text_size(SIDEBAR_ROW_FONT_SIZE)
        });
        input.update(cx, |field, input_cx| {
            field.set_bare(true, input_cx);
            field.set_right_padding(0., input_cx);
            field.select_all(input_cx);
        });
        let focus = input.read(cx).focus_handle();
        let root = cx.entity();
        self._rename_focus_subscription =
            Some(cx.on_focus_out(&focus, window, move |_, _, window, cx| {
                root.update(cx, |this, cx| this.submit_sidebar_rename(window, cx));
            }));
        self.rename = Some(SidebarRename { target, input });
        focus.focus(window);
        cx.notify();
    }

    fn submit_sidebar_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = self.rename.take() else {
            return;
        };
        self._rename_focus_subscription = None;
        let next = rename.input.read(cx).text().trim().to_owned();
        let result = match rename.target {
            SidebarRenameTarget::Tab { node_id, original } => {
                if next == original.trim() {
                    Ok(())
                } else {
                    runner_backend::ops::node::node_rename(self.core(cx), node_id, next).map(drop)
                }
            }
            SidebarRenameTarget::Project {
                project_id,
                original,
            } => {
                if next.is_empty() || next == original.trim() {
                    Ok(())
                } else {
                    runner_backend::ops::project::project_rename(self.core(cx), project_id, next)
                        .map(drop)
                }
            }
            SidebarRenameTarget::Mission {
                mission_id,
                original,
            } => {
                if next.is_empty() || next == original.trim() {
                    Ok(())
                } else {
                    futures::executor::block_on(runner_backend::ops::mission::mission_rename_impl(
                        self.core(cx),
                        mission_id,
                        next,
                    ))
                    .map(drop)
                }
            }
        };
        match result {
            Ok(()) => self.refresh_store(StoreRefreshKind::All, cx),
            Err(error) => self.report_error(error.to_string(), cx),
        }
        self.focus_shell_terminal(window, cx);
        cx.notify();
    }

    fn cancel_sidebar_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.rename = None;
        self._rename_focus_subscription = None;
        self.focus_shell_terminal(window, cx);
        cx.notify();
    }

    fn open_sidebar_context_menu(
        &mut self,
        position: gpui::Point<gpui::Pixels>,
        width: f32,
        entries: Vec<(UiMenuItem, SidebarMenuAction)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = entries
            .iter()
            .map(|(item, _)| item.clone())
            .collect::<Vec<_>>();
        let actions = entries
            .into_iter()
            .map(|(_, action)| action)
            .collect::<Vec<_>>();
        let root = cx.entity();
        let dismiss_root = root.clone();
        let menu = cx.new(move |menu_cx| {
            let action_root = root.clone();
            ContextMenu::new(
                "sidebar-context-menu",
                menu_cx.focus_handle(),
                position,
                items,
                Rc::new(move |index, window, cx| {
                    if let Some(action) = actions.get(index).cloned() {
                        action_root.update(cx, |this, cx| {
                            this.handle_sidebar_menu_action(action, window, cx)
                        });
                    }
                }),
                Rc::new(move |_, cx| {
                    dismiss_root.update(cx, |this, cx| {
                        this.context_menu = None;
                        this.schedule_shell_notify(cx);
                        cx.notify();
                    });
                }),
            )
            .width(px(width))
        });
        let focus = menu.read(cx).focus_handle();
        self.context_menu = Some(menu);
        focus.focus(window);
        self.schedule_shell_notify(cx);
        cx.notify();
    }

    fn open_tab_menu(
        &mut self,
        node: NodeRow,
        layout: PaneLayout,
        members: Vec<DirectSessionEntry>,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pinned = node.pinned_position.is_some();
        let mut entries = vec![
            (
                UiMenuItem::new(if pinned { "Unpin" } else { "Pin" }).icon(if pinned {
                    "pin-off.svg"
                } else {
                    "pin.svg"
                }),
                SidebarMenuAction::TogglePin {
                    node_id: node.id.clone(),
                    pinned,
                },
            ),
            (
                UiMenuItem::new("Rename tab").icon("pencil.svg"),
                SidebarMenuAction::Rename(SidebarRenameTarget::Tab {
                    node_id: node.id.clone(),
                    original: layout.name.clone().unwrap_or_default(),
                }),
            ),
            (
                UiMenuItem::new("Open in New Window").icon("app-window.svg"),
                SidebarMenuAction::OpenInNewWindow(format!(
                    "/chats/{}",
                    layout
                        .focused_session_id()
                        .unwrap_or(&members[0].session_id)
                )),
            ),
        ];
        if node_project_id(&self.app_store.read(cx).nodes, &node).is_some() {
            entries.push((
                UiMenuItem::new("Remove from project").icon("folder-minus.svg"),
                SidebarMenuAction::RemoveTabFromProject(
                    members
                        .iter()
                        .map(|member| member.session_id.clone())
                        .collect(),
                ),
            ));
        }
        entries.push((
            UiMenuItem::new(if layout.root.leaves().len() > 1 {
                "Archive all"
            } else {
                "Archive"
            })
            .icon("archive.svg")
            .destructive(true),
            SidebarMenuAction::ArchiveTab(
                members
                    .iter()
                    .map(|member| member.session_id.clone())
                    .collect(),
            ),
        ));
        self.open_sidebar_context_menu(position, 160., entries, window, cx);
    }

    fn open_mission_menu(
        &mut self,
        node: NodeRow,
        summary: MissionSummary,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pinned = node.pinned_position.is_some();
        let mut entries = vec![
            (
                UiMenuItem::new(if pinned { "Unpin" } else { "Pin" }).icon(if pinned {
                    "pin-off.svg"
                } else {
                    "pin.svg"
                }),
                SidebarMenuAction::TogglePin {
                    node_id: node.id,
                    pinned,
                },
            ),
            (
                UiMenuItem::new("Rename").icon("pencil.svg"),
                SidebarMenuAction::Rename(SidebarRenameTarget::Mission {
                    mission_id: summary.mission.id.clone(),
                    original: summary.mission.title.clone(),
                }),
            ),
            (
                UiMenuItem::new("Open in New Window").icon("app-window.svg"),
                SidebarMenuAction::OpenInNewWindow(format!("/missions/{}", summary.mission.id)),
            ),
        ];
        if summary.mission.project_id.is_some() {
            entries.push((
                UiMenuItem::new("Remove from project").icon("folder-minus.svg"),
                SidebarMenuAction::RemoveMissionFromProject(summary.mission.id.clone()),
            ));
        }
        entries.push((
            UiMenuItem::new("Archive")
                .icon("archive.svg")
                .destructive(true),
            SidebarMenuAction::ArchiveMission(summary.mission.id),
        ));
        self.open_sidebar_context_menu(position, 160., entries, window, cx);
    }

    fn open_project_menu(
        &mut self,
        project: runner_backend::repo::project::ProjectRow,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entries = vec![
            (
                UiMenuItem::new("New chat in project").icon("message-square-plus.svg"),
                SidebarMenuAction::NewChat(Some(project.id.clone())),
            ),
            (
                UiMenuItem::new("New mission in project").icon("flag.svg"),
                SidebarMenuAction::NewMission(Some(project.id.clone())),
            ),
            (
                UiMenuItem::new("Rename project")
                    .icon("pencil.svg")
                    .separator_before(true),
                SidebarMenuAction::Rename(SidebarRenameTarget::Project {
                    project_id: project.id.clone(),
                    original: project.name,
                }),
            ),
            (
                UiMenuItem::new("Delete project")
                    .icon("trash.svg")
                    .separator_before(true)
                    .destructive(true),
                SidebarMenuAction::DeleteProject(project.id),
            ),
        ];
        self.open_sidebar_context_menu(position, 200., entries, window, cx);
    }

    fn handle_sidebar_menu_action(
        &mut self,
        action: SidebarMenuAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            SidebarMenuAction::NewChat(project_id) => {
                self.set_active_project(project_id.clone(), cx);
                if let Some(project_id) = project_id.as_deref() {
                    let project_id = project_id.to_owned();
                    self.update_app_settings(cx, true, move |settings| {
                        settings.sidebar_projects_open = true;
                        settings.sidebar_collapsed_projects.remove(&project_id);
                        true
                    });
                }
                if let Some(shell) = self.shell.upgrade() {
                    window.defer(cx, move |window, cx| {
                        shell.update(cx, |shell, shell_cx| {
                            shell.open_sidebar_chat_modal(project_id.as_deref(), window, shell_cx)
                        });
                    });
                }
            }
            SidebarMenuAction::NewMission(project_id) => {
                self.set_active_project(project_id.clone(), cx);
                if let Some(shell) = self.shell.upgrade() {
                    window.defer(cx, move |window, cx| {
                        shell.update(cx, |shell, shell_cx| {
                            shell.open_start_mission_modal(None, project_id, window, shell_cx)
                        });
                    });
                }
            }
            SidebarMenuAction::OpenInNewWindow(route) => {
                cx.defer(move |cx| {
                    if let Err(error) = open_new_runner_window(Some(route), cx) {
                        eprintln!("Runner new window failed: {error:#}");
                    }
                    cx.activate(true);
                });
            }
            SidebarMenuAction::TogglePin { node_id, pinned } => {
                match runner_backend::ops::node::node_set_pinned(self.core(cx), node_id, !pinned) {
                    Ok(_) => self.refresh_store(StoreRefreshKind::All, cx),
                    Err(error) => self.report_error(error.to_string(), cx),
                }
            }
            SidebarMenuAction::Rename(target) => {
                let (value, placeholder) = match &target {
                    SidebarRenameTarget::Tab { original, node_id } => {
                        let placeholder = self
                            .shell
                            .upgrade()
                            .and_then(|shell| {
                                let shell = shell.read(cx);
                                shell
                                    .tabs
                                    .tabs()
                                    .iter()
                                    .find(|layout| layout.id == *node_id)
                                    .map(|layout| shell.tab_label(layout, cx))
                            })
                            .unwrap_or_else(|| "Chat tab".into());
                        (original.clone(), placeholder)
                    }
                    SidebarRenameTarget::Project { original, .. }
                    | SidebarRenameTarget::Mission { original, .. } => {
                        (original.clone(), original.clone())
                    }
                };
                self.begin_sidebar_rename(target, value, placeholder, window, cx);
            }
            SidebarMenuAction::RemoveTabFromProject(session_ids) => {
                match runner_backend::ops::session::session_set_project(
                    self.core(cx),
                    session_ids,
                    None,
                ) {
                    Ok(()) => self.refresh_store(StoreRefreshKind::All, cx),
                    Err(error) => self.report_error(error.to_string(), cx),
                }
            }
            SidebarMenuAction::RemoveMissionFromProject(mission_id) => {
                match runner_backend::ops::mission::mission_set_project(
                    self.core(cx),
                    &mission_id,
                    None,
                ) {
                    Ok(_) => self.refresh_store(StoreRefreshKind::All, cx),
                    Err(error) => self.report_error(error.to_string(), cx),
                }
            }
            SidebarMenuAction::ArchiveTab(session_ids) => {
                self.archive_sidebar_tab(session_ids, window, cx)
            }
            SidebarMenuAction::ArchiveMission(mission_id) => {
                if !self.archiving_missions.insert(mission_id.clone()) {
                    return;
                }
                cx.notify();
                let core = self.core(cx).clone();
                let archive_id = mission_id.clone();
                let archive_task = cx.background_spawn(async move {
                    runner_backend::ops::mission::mission_archive_impl(&core, archive_id)
                        .await
                        .map(drop)
                        .map_err(|error| error.to_string())
                });
                cx.spawn(async move |weak, cx| {
                    let result = archive_task.await;
                    let _ = weak.update(cx, |this, cx| {
                        this.archiving_missions.remove(&mission_id);
                        match result {
                            Ok(()) => this.core(cx).events.emit("mission/changed", &()),
                            Err(error) => this.report_error(error, cx),
                        }
                        this.refresh_store(StoreRefreshKind::All, cx);
                    });
                })
                .detach();
            }
            SidebarMenuAction::DeleteProject(project_id) => {
                if let Some(shell) = self.shell.upgrade() {
                    shell.update(cx, |shell, shell_cx| {
                        shell.project_delete_confirm = Some(project_id);
                        shell_cx.notify();
                    });
                }
            }
        }
    }

    pub(crate) fn archive_sidebar_tab(
        &mut self,
        session_ids: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = self
            .shell
            .upgrade()
            .and_then(|shell| shell.read(cx).active_focused_session_id());
        self.archive_sessions(session_ids, ArchiveErrorTarget::App, active, window, cx);
    }

    fn archive_sessions(
        &mut self,
        mut session_ids: Vec<String>,
        error_target: ArchiveErrorTarget,
        active: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if session_ids
            .iter()
            .any(|id| self.archiving_sessions.contains(id))
        {
            return;
        }
        session_ids.sort_by_key(|id| active.as_deref() == Some(id.as_str()));
        let entries = session_ids
            .iter()
            .map(|id| {
                (
                    id.clone(),
                    self.app_store
                        .read(cx)
                        .sessions
                        .iter()
                        .find(|session| session.session_id == *id)
                        .is_some_and(|session| session.status == SessionStatus::Running),
                )
            })
            .collect::<Vec<_>>();
        self.archiving_sessions.extend(session_ids.iter().cloned());
        self.schedule_shell_notify(cx);
        cx.notify();

        let core = self.core(cx).clone();
        let pending_ids = session_ids;
        let archive_task = cx.background_spawn(async move {
            let mut archived = Vec::new();
            for (session_id, running) in entries {
                if running {
                    let _ = runner_backend::ops::session::session_kill(&core, &session_id);
                }
                if let Err(error) =
                    runner_backend::ops::session::session_archive(&core, &session_id)
                {
                    return (archived, Some(error.to_string()));
                }
                archived.push(session_id);
            }
            (archived, None)
        });
        cx.spawn_in(window, async move |weak, cx| {
            let (archived, archive_error) = archive_task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                for session_id in &pending_ids {
                    this.archiving_sessions.remove(session_id);
                }
                cx.notify();
                if let Some(shell) = this.shell.upgrade() {
                    window.defer(cx, move |window, cx| {
                        shell.update(cx, |shell, shell_cx| {
                            shell.finish_sidebar_archive(
                                archived,
                                archive_error,
                                error_target,
                                window,
                                shell_cx,
                            )
                        });
                    });
                }
            });
        })
        .detach();
    }
}

impl NativeRoot {
    fn finish_sidebar_archive(
        &mut self,
        archived: Vec<String>,
        archive_error: Option<String>,
        error_target: ArchiveErrorTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for session_id in archived {
            self.attached.remove(&session_id);
        }
        let refresh_result = (|| -> Result<()> {
            self.refresh_sessions(cx);
            self.reload_tabs(cx)?;
            self.ensure_active_tab_attached(window, cx)?;
            Ok(())
        })();
        if refresh_result.is_ok() {
            self.mark_active_tab_viewed(window, cx);
            self.focus_active_terminal(window, cx);
        }
        let error = archive_error.or_else(|| {
            refresh_result
                .err()
                .map(|refresh_error| refresh_error.to_string())
        });
        match error_target {
            ArchiveErrorTarget::App => self.error = error,
            ArchiveErrorTarget::Chat => self.chat_error = error,
        }
        cx.notify();
    }

    fn open_project_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = self.settings(cx).default_working_dir.trim().to_owned();
        let name = project_name_from_path(&cwd);
        let cwd_input = cx.new(|input_cx| {
            working_dir_text_field(input_cx.focus_handle(), cwd, "/Users/you/projects/runner")
                .text_size(12.)
        });
        let name_input = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), name, "runner", false).text_size(13.)
        });
        let watched_name = name_input.clone();
        self._project_cwd_subscription =
            Some(cx.observe(&cwd_input, move |this, cwd_input, cx| {
                let Some(modal) = this.project_modal.as_ref() else {
                    return;
                };
                if modal.name != watched_name || modal.name.read(cx).edited() {
                    return;
                }
                let derived = project_name_from_path(cwd_input.read(cx).text());
                watched_name.update(cx, |name, name_cx| name.reset(derived, name_cx));
            }));
        let cwd_focus = cwd_input.read(cx).focus_handle();
        self.project_modal = Some(ProjectModal {
            cwd: cwd_input,
            name: name_input,
            browse_focus: cx.focus_handle(),
            close_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            submit_focus: cx.focus_handle(),
            error: None,
            submitting: false,
        });
        self.update_app_settings(cx, true, |settings| {
            settings.sidebar_projects_open = true;
            true
        });
        cwd_focus.focus(window);
        cx.notify();
    }

    fn browse_project_cwd(&mut self, cx: &mut Context<Self>) {
        let Some(cwd_input) = self.project_modal.as_ref().map(|modal| modal.cwd.clone()) else {
            return;
        };
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Pick a project directory".into()),
        });
        cx.spawn(async move |weak, cx| {
            let result = selected
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result.map_err(|error| error.to_string()));
            let _ = weak.update(cx, |this, cx| {
                let Some(modal) = this.project_modal.as_mut() else {
                    return;
                };
                if modal.cwd != cwd_input {
                    return;
                }
                match result {
                    Ok(Some(paths)) => {
                        if let Some(path) = paths.into_iter().next() {
                            modal.cwd.update(cx, |field, field_cx| {
                                field.reset(path.to_string_lossy().into_owned(), field_cx)
                            });
                        }
                    }
                    Ok(None) => {}
                    Err(error) => modal.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn close_project_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .project_modal
            .as_ref()
            .is_some_and(|modal| modal.submitting)
        {
            return;
        }
        self.project_modal = None;
        self._project_cwd_subscription = None;
        self.focus_active_terminal(window, cx);
        cx.notify();
    }

    fn submit_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.project_modal.as_mut() else {
            return;
        };
        let cwd = modal.cwd.read(cx).text().trim().to_owned();
        let name = modal.name.read(cx).text().trim().to_owned();
        if cwd.is_empty() || name.is_empty() || modal.submitting {
            return;
        }
        modal.submitting = true;
        modal.error = None;
        match runner_backend::ops::project::project_create(self.core(cx), name, cwd) {
            Ok(project) => {
                self.project_modal = None;
                self._project_cwd_subscription = None;
                self.refresh_store(StoreRefreshKind::All, cx);
                self.sidebar.update(cx, |sidebar, sidebar_cx| {
                    sidebar.set_active_project(Some(project.id), sidebar_cx)
                });
                self.focus_active_terminal(window, cx);
            }
            Err(error) => {
                if let Some(modal) = self.project_modal.as_mut() {
                    modal.submitting = false;
                    modal.error = Some(error.to_string());
                }
            }
        }
        cx.notify();
    }

    fn on_project_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "enter"
            && self.project_modal.as_ref().is_some_and(|modal| {
                !modal.cwd.read(cx).is_composing() && !modal.name.read(cx).is_composing()
            })
        {
            cx.stop_propagation();
            self.submit_project(window, cx);
        }
    }

    fn confirm_delete_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project_id) = self.project_delete_confirm.clone() else {
            return;
        };
        let deleting_active_chat = self
            .tabs
            .active_tab_id()
            .and_then(|tab_id| {
                self.app_store
                    .read(cx)
                    .nodes
                    .iter()
                    .find(|node| node.id == tab_id)
            })
            .and_then(|node| node_project_id(&self.app_store.read(cx).nodes, node))
            .as_deref()
            == Some(project_id.as_str());
        if deleting_active_chat {
            self.set_route(AppRoute::Runners, cx);
            self.load_runner_page(cx);
            window.focus(&self.root_focus);
        }
        self.project_delete_busy = true;
        let core = self.core(cx).clone();
        let deleting_project_id = project_id.clone();
        cx.spawn(async move |weak, cx| {
            let result = runner_backend::ops::project::project_delete(&core, project_id).await;
            let _ = weak.update(cx, |this, cx| {
                this.project_delete_busy = false;
                if let Err(error) = result {
                    this.error = Some(error.to_string());
                } else {
                    this.project_delete_confirm = None;
                    this.sidebar.update(cx, |sidebar, sidebar_cx| {
                        if sidebar.active_project_id.as_deref()
                            == Some(deleting_project_id.as_str())
                        {
                            sidebar.set_active_project(None, sidebar_cx);
                        }
                    });
                }
                this.refresh_store(StoreRefreshKind::All, cx);
            });
        })
        .detach();
    }
}

impl Sidebar {
    pub(crate) fn clear_sidebar_drag(&mut self, cx: &mut Context<Self>) {
        if self.dragged_id.take().is_some()
            || self.drop_target.take().is_some()
            || self.drop_marker.take().is_some()
        {
            cx.notify();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn update_row_drop_target(
        &mut self,
        dragged_id: &str,
        kind: DropKind,
        parent_id: Option<&str>,
        visible_ids: &[String],
        hovered_id: &str,
        after: bool,
        marker: String,
        cx: &mut Context<Self>,
    ) {
        self.dragged_id = Some(dragged_id.to_owned());
        self.drop_target = list_drop_target(
            &self.app_store.read(cx).nodes,
            kind,
            parent_id,
            visible_ids,
            dragged_id,
            hovered_id,
            after,
        );
        self.drop_marker = self.drop_target.as_ref().map(|_| marker);
        cx.notify();
    }

    fn update_project_container_drop_target(
        &mut self,
        dragged_id: &str,
        project_node_id: &str,
        visible_ids: &[String],
        cx: &mut Context<Self>,
    ) {
        self.dragged_id = Some(dragged_id.to_owned());
        self.drop_target = container_drop_target(
            &self.app_store.read(cx).nodes,
            visible_ids,
            dragged_id,
            project_node_id,
        );
        self.drop_marker = self
            .drop_target
            .as_ref()
            .map(|_| format!("container:{project_node_id}"));
        cx.notify();
    }

    fn commit_sidebar_drop(&mut self, dragged_id: &str, cx: &mut Context<Self>) {
        if self.dragged_id.as_deref() != Some(dragged_id) {
            self.clear_sidebar_drag(cx);
            return;
        }
        let Some(target) = self.drop_target.clone() else {
            self.clear_sidebar_drag(cx);
            return;
        };
        let rows = self.resolved_sidebar_rows(cx);
        let result = match target.kind {
            DropKind::Pinned => {
                let visible = rows
                    .iter()
                    .filter(|row| row.node().pinned_position.is_some())
                    .map(|row| row.node().id.clone())
                    .collect::<Vec<_>>();
                let order = ordered_pinned_node_ids_after_drop(
                    &self.app_store.read(cx).nodes,
                    &visible,
                    dragged_id,
                    target.index,
                );
                runner_backend::ops::node::node_reorder_pinned(self.core(cx), order)
            }
            DropKind::Project => {
                let order = ordered_root_node_ids_after_project_drop(
                    &self.app_store.read(cx).nodes,
                    dragged_id,
                    target.index,
                );
                runner_backend::ops::node::node_move(
                    self.core(cx),
                    dragged_id.to_owned(),
                    None,
                    order,
                )
            }
            DropKind::Leaf => {
                let visible = self
                    .scope_rows(&rows, target.parent_id.as_deref())
                    .into_iter()
                    .map(|row| row.node().id.clone())
                    .collect::<Vec<_>>();
                let visible =
                    ordered_visible_node_ids_after_drop(&visible, dragged_id, target.index);
                let order = complete_unpinned_scope_order(
                    &self.app_store.read(cx).nodes,
                    target.parent_id.as_deref(),
                    dragged_id,
                    &visible,
                );
                runner_backend::ops::node::node_move(
                    self.core(cx),
                    dragged_id.to_owned(),
                    target.parent_id,
                    order,
                )
            }
        };
        match result {
            Ok(nodes) => {
                if let Some(shell) = self.shell.upgrade() {
                    shell.update(cx, |shell, shell_cx| {
                        if let Err(error) = shell.tabs.replace_rows(&nodes) {
                            shell.error = Some(error.to_string());
                            shell_cx.notify();
                        }
                    });
                }
                self.app_store
                    .update(cx, |store, store_cx| store.replace_nodes(nodes, store_cx));
                self.refresh_store(StoreRefreshKind::All, cx);
            }
            Err(error) => self.report_error(error.to_string(), cx),
        }
        self.clear_sidebar_drag(cx);
    }

    pub(crate) fn render_sidebar_contents(&self, cx: &mut Context<Self>) -> AnyElement {
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

        let mut scroll = sidebar_scroll_container("sidebar-node-scroll", &self.scroll);
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
                                self.render_sidebar_row(
                                    row,
                                    project_id,
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
                                        .text_size(rems(10. / 16.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("RECENT"),
                                )
                                .child(
                                    svg()
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
                                    self.render_sidebar_row(
                                        row,
                                        None,
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
        div()
            .min_h(px(0.))
            .flex_1()
            .flex()
            .flex_col()
            .overflow_hidden()
            .pb_3()
            .child(
                div().flex_none().child(section_title("WORKSPACE")).child(
                    div()
                        .px_3()
                        .pb_1()
                        .flex()
                        .flex_col()
                        .gap(rems(2. / 16.))
                        .child(workspace_row(
                            "workspace-runner",
                            "terminal.svg",
                            "runner",
                            matches!(&route, AppRoute::Runners | AppRoute::RunnerDetail(_)),
                            {
                                let shell = self.shell.clone();
                                move |window, cx| {
                                    if let Some(shell) = shell.upgrade() {
                                        shell.update(cx, |shell, shell_cx| {
                                            shell.open_runners(window, shell_cx)
                                        });
                                    }
                                }
                            },
                        ))
                        .child(workspace_row(
                            "workspace-crew",
                            "users.svg",
                            "crew",
                            matches!(&route, AppRoute::Crews | AppRoute::CrewEditor(_)),
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
                        )),
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
                            .text_size(rems(10. / 16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(label),
                    )
                    .child(
                        svg()
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
    fn render_sidebar_row(
        &self,
        row: SidebarRow,
        project_id: Option<String>,
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
                drop_kind,
                parent_id,
                visible_ids,
                cx,
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_tab_row(
        &self,
        node: NodeRow,
        layout: PaneLayout,
        members: Vec<DirectSessionEntry>,
        attention: AttentionState,
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
            members
                .iter()
                .map(session_label)
                .collect::<Vec<_>>()
                .join(" + ")
        });
        let focused = layout
            .focused_session_id()
            .or_else(|| members.first().map(|member| member.session_id.as_str()))
            .unwrap_or_default()
            .to_owned();
        let renaming = self
            .rename
            .as_ref()
            .is_some_and(|rename| rename.target.matches(NodeType::Tab, &node.id));
        let target = members
            .iter()
            .find(|member| member.session_id == focused)
            .unwrap_or(&members[0]);
        let click_tab = node.id.clone();
        let click_session = target.session_id.clone();
        let leaf_icon = if pane_count >= 3 {
            "columns-3.svg"
        } else if pane_count > 1 {
            "columns-2.svg"
        } else {
            "message-square.svg"
        };
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
                cx,
            )
        } else {
            sidebar_row_shell(
                SharedString::from(format!("sidebar-tab-{}", node.id)),
                active,
                false,
            )
            .children(node.pinned_position.is_some().then(pin_indicator))
            .child(sidebar_icon(leaf_icon, live))
            .child(sidebar_row_label(label.clone(), active, false))
            .child(attention_indicator(attention))
            .child(
                IconButton::new(
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
                }),
            )
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
    fn render_mission_row(
        &self,
        node: NodeRow,
        summary: MissionSummary,
        attention: AttentionState,
        project_id: Option<String>,
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
                ("flag.svg", summary.all_sessions_live),
                attention,
                cx,
            )
        } else {
            sidebar_row_shell(
                SharedString::from(format!("sidebar-mission-{}", summary.mission.id)),
                active,
                false,
            )
            .children(node.pinned_position.is_some().then(pin_indicator))
            .child(sidebar_icon("flag.svg", summary.all_sessions_live))
            .child(sidebar_row_label(label.clone(), active, false))
            .child(attention_indicator(attention))
            .child(
                IconButton::new(
                    SharedString::from(format!("sidebar-mission-actions-{}", summary.mission.id)),
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

    fn render_project(
        &self,
        node: NodeRow,
        project: runner_backend::repo::project::ProjectRow,
        nested: Vec<SidebarRow>,
        attention: AttentionState,
        visible_projects: Vec<String>,
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
            .children(collapsed.then(|| attention_indicator(attention)))
            .child(
                IconButton::new(
                    SharedString::from(format!("sidebar-project-actions-{}", project.id)),
                    "more-horizontal.svg",
                )
                .size(IconButtonSize::Xs)
                .stop_click_propagation(true)
                .reveal_on_group_hover("sidebar-row-actions")
                .tooltip("Project actions")
                .on_press(move |window, cx| {
                    let position = window.mouse_position();
                    menu_root.update(cx, |this, cx| {
                        this.open_project_menu(menu_project.clone(), position, window, cx)
                    });
                }),
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
        let mut header_wrap = div()
            .id(SharedString::from(format!("project-drag-wrap-{}", node.id)))
            .relative()
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
                            this.dragged_id = Some(drag.node_id.clone());
                            cx.notify();
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
                                self.render_sidebar_row(
                                    row,
                                    Some(project_id.clone()),
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

    fn render_end_drop_divider(
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
        let mut wrapper = div()
            .id(SharedString::from(format!("sidebar-drag-{}", node.id)))
            .relative()
            .cursor_move()
            .child(row)
            .on_drag(drag, move |drag: &SidebarNodeDrag, _, _, cx| {
                drag_root.update(cx, |this, cx| {
                    this.dragged_id = Some(drag.node_id.clone());
                    cx.notify();
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

    fn render_inline_rename_row(
        &self,
        kind: NodeType,
        id: &str,
        disclosure_icon: Option<&'static str>,
        icon: (&'static str, bool),
        attention: AttentionState,
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

impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_sidebar_contents(cx)
    }
}

impl NativeRoot {
    pub(crate) fn render_sidebar_overlays(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut overlays = Vec::new();
        if let Some(menu) = &self.sidebar.read(cx).context_menu {
            overlays.push(menu.clone().into_any_element());
        }
        if self.project_modal.is_some() {
            overlays.push(self.render_project_modal(cx));
        }
        if let Some(project_id) = &self.project_delete_confirm {
            if let Some(project) = self
                .app_store
                .read(cx)
                .projects
                .iter()
                .find(|project| project.id == *project_id)
                .cloned()
            {
                let confirm_root = cx.entity();
                let cancel_root = confirm_root.clone();
                overlays.push(
                    ConfirmDialog::new(
                        format!("Delete project \"{}\"?", project.name),
                        "Deleting this project archives every chat and mission inside it (running ones are stopped first). Archived items appear in Settings → Archived. The on-disk directory and all of its files remain untouched.",
                        "Delete project",
                        "Archiving…",
                        self.project_delete_busy,
                        Rc::new(move |window, cx| {
                            confirm_root.update(cx, |this, cx| {
                                this.confirm_delete_project(window, cx)
                            });
                        }),
                        Rc::new(move |_, cx| {
                            cancel_root.update(cx, |this, cx| {
                                if !this.project_delete_busy {
                                    this.project_delete_confirm = None;
                                    cx.notify();
                                }
                            });
                        }),
                    )
                    .into_any_element(),
                );
            }
        }
        overlays
    }

    fn render_project_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let modal = self.project_modal.as_ref().expect("project modal");
        let submitting = modal.submitting;
        let can_create = !modal.cwd.read(cx).text().trim().is_empty()
            && !modal.name.read(cx).text().trim().is_empty()
            && !submitting;
        let root = cx.entity();
        let close_root = root.clone();
        let cancel_root = root.clone();
        let browse_root = root.clone();
        let submit_root = root.clone();
        let modal_close_root = root;
        let title = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(2. / 16.))
                    .child(
                        div()
                            .text_size(rems(1.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Start project"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme::muted())
                            .child("Add a named working directory to the sidebar."),
                    ),
            )
            .child(
                IconButton::new("close-project-modal", "close.svg")
                    .focus_handle(modal.close_focus.clone())
                    .tooltip("Close start project")
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        close_root.update(cx, |this, cx| this.close_project_modal(window, cx));
                    }),
            );
        let mut body = div()
            .flex()
            .flex_col()
            .gap_5()
            .on_key_down(cx.listener(Self::on_project_key_down))
            .children(modal.error.as_ref().map(|error| {
                div()
                    .rounded_sm()
                    .border_1()
                    .border_color(theme::with_alpha(theme::danger(), 0.4))
                    .bg(theme::with_alpha(theme::danger(), 0.1))
                    .px_3()
                    .py_2()
                    .text_xs()
                    .text_color(theme::danger())
                    .child(error.clone())
            }))
            .child(
                Field::new(
                    "project-directory",
                    "Directory",
                    WorkingDirField::new(
                        modal.cwd.clone(),
                        submitting,
                        Rc::new(move |_, cx| {
                            browse_root.update(cx, |this, cx| this.browse_project_cwd(cx));
                        }),
                    )
                    .browse_focus(modal.browse_focus.clone()),
                )
                .focus_target(modal.cwd.read(cx).focus_handle())
                .emphasized(true),
            )
            .child(
                Field::new("project-name", "Name", modal.name.clone())
                    .focus_target(modal.name.read(cx).focus_handle())
                    .emphasized(true),
            );
        if submitting {
            body = body.opacity(0.7);
        }
        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("cancel-project", "Cancel")
                    .focus_handle(modal.cancel_focus.clone())
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        cancel_root.update(cx, |this, cx| this.close_project_modal(window, cx));
                    }),
            )
            .child(
                Button::new(
                    "create-project",
                    if submitting {
                        "Creating…"
                    } else {
                        "Create project"
                    },
                )
                .focus_handle(modal.submit_focus.clone())
                .variant(ButtonVariant::Primary)
                .disabled(!can_create)
                .on_press(move |window, cx| {
                    submit_root.update(cx, |this, cx| this.submit_project(window, cx));
                }),
            );
        Modal::new(
            title,
            body,
            Rc::new(move |window, cx| {
                modal_close_root.update(cx, |this, cx| this.close_project_modal(window, cx));
            }),
        )
        .width(OverlayWidth::Custom(560.))
        .busy(submitting)
        .focus_order(if submitting {
            Vec::new()
        } else {
            vec![
                modal.cwd.read(cx).focus_handle(),
                modal.browse_focus.clone(),
                modal.name.read(cx).focus_handle(),
                modal.cancel_focus.clone(),
                modal.submit_focus.clone(),
                modal.close_focus.clone(),
            ]
        })
        .footer(footer)
        .into_any_element()
    }
}

fn node_project_id(nodes: &[NodeRow], node: &NodeRow) -> Option<String> {
    let parent = nodes
        .iter()
        .find(|candidate| node.parent_id.as_deref() == Some(candidate.id.as_str()))?;
    (parent.node_type == NodeType::Project)
        .then(|| parent.ref_id.clone())
        .flatten()
}

fn project_name_from_path(path: &str) -> String {
    Path::new(path.trim())
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned()
}

fn section_title(label: &'static str) -> AnyElement {
    div()
        .px_5()
        .pb_2()
        .text_size(rems(10. / 16.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::faint())
        .child(label)
        .into_any_element()
}

fn workspace_row(
    id: &'static str,
    icon: &'static str,
    label: &'static str,
    active: bool,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let on_click = Rc::new(on_click);
    let key_click = Rc::clone(&on_click);
    div()
        .id(id)
        .tab_index(0)
        .px(rems(10. / 16.))
        .py(rems(6. / 16.))
        .flex()
        .items_center()
        .gap_2()
        .rounded_sm()
        .border_1()
        .border_color(if active {
            theme::sidebar_selected_border()
        } else {
            gpui::transparent_black()
        })
        .when(active, |row| row.bg(theme::sidebar_selected()).shadow_sm())
        .cursor_pointer()
        .text_sm()
        .font_weight(if active {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        })
        .text_color(if active {
            theme::text()
        } else {
            theme::muted()
        })
        .hover(|row| {
            row.border_color(theme::sidebar_selected_border())
                .bg(theme::with_alpha(theme::sidebar_selected(), 0.4))
                .text_color(theme::text())
        })
        .focus_visible(|row| {
            row.border_color(theme::sidebar_selected_border())
                .bg(theme::with_alpha(theme::sidebar_selected(), 0.4))
                .text_color(theme::text())
        })
        .child(
            svg()
                .path(icon)
                .size(rems(12. / 16.))
                .flex_none()
                .text_color(if active {
                    theme::text()
                } else {
                    theme::muted()
                }),
        )
        .child(label)
        .on_click(move |_, window, cx| on_click(window, cx))
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.stop_propagation();
                key_click(window, cx);
            }
        })
        .into_any_element()
}

fn empty_sidebar_label(label: &'static str) -> AnyElement {
    div()
        .px(rems(10. / 16.))
        .py_1()
        .text_size(rems(SIDEBAR_ROW_FONT_SIZE / 16.))
        .text_color(theme::faint())
        .child(label)
        .into_any_element()
}

fn sidebar_row_shell(
    id: SharedString,
    selected: bool,
    accent_bar: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .relative()
        .group("sidebar-row-actions")
        .w_full()
        .px(rems(10. / 16.))
        .py(rems(6. / 16.))
        .flex()
        .items_center()
        .gap(rems(6. / 16.))
        .rounded_sm()
        .border_1()
        .border_color(if selected {
            theme::sidebar_selected_border()
        } else {
            gpui::transparent_black()
        })
        .when(selected, |row| row.bg(theme::sidebar_selected()))
        .when(accent_bar, |row| {
            row.child(
                div()
                    .absolute()
                    .left_0()
                    .top(rems(2. / 16.))
                    .bottom(rems(2. / 16.))
                    .w(rems(2. / 16.))
                    .rounded_full()
                    .bg(theme::accent()),
            )
        })
        .cursor_pointer()
        .text_size(rems(SIDEBAR_ROW_FONT_SIZE / 16.))
        .text_color(if selected {
            theme::text()
        } else {
            theme::muted()
        })
        .hover(|row| {
            row.border_color(theme::sidebar_selected_border())
                .bg(theme::with_alpha(theme::sidebar_selected(), 0.4))
                .text_color(theme::text())
        })
}

fn sidebar_row_label(label: String, selected: bool, monospace: bool) -> AnyElement {
    div()
        .min_w(px(0.))
        .flex_1()
        .overflow_hidden()
        .whitespace_nowrap()
        .when(monospace, |label| label.font_family("Menlo"))
        .font_weight(if selected {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        })
        .child(label)
        .into_any_element()
}

fn chat_tab_row_active(route: &AppRoute, active_tab_id: Option<&str>, node_id: &str) -> bool {
    matches!(route, AppRoute::Chat) && active_tab_id == Some(node_id)
}

fn project_row_label(label: String) -> AnyElement {
    div()
        .min_w(px(0.))
        .flex_1()
        .overflow_hidden()
        .whitespace_nowrap()
        .font_weight(FontWeight::MEDIUM)
        .child(label)
        .into_any_element()
}

fn sidebar_icon(path: &'static str, active: bool) -> AnyElement {
    svg()
        .path(path)
        .size(rems(12. / 16.))
        .flex_none()
        .text_color(if active {
            theme::accent()
        } else {
            theme::muted()
        })
        .into_any_element()
}

fn pin_indicator() -> AnyElement {
    svg()
        .path("pin.svg")
        .size(rems(10. / 16.))
        .flex_none()
        .text_color(theme::faint())
        .with_transformation(Transformation::rotate(radians(
            -std::f32::consts::FRAC_PI_4,
        )))
        .into_any_element()
}

fn attention_indicator(attention: AttentionState) -> AnyElement {
    let slot = div()
        .size(rems(12. / 16.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center();
    match attention {
        AttentionState::Working => slot
            .child(runner_app::ui::button::spinner(
                "sidebar-working-indicator",
                12.,
                theme::faint(),
            ))
            .into_any_element(),
        AttentionState::Unread => slot
            .child(
                div()
                    .size(rems(6. / 16.))
                    .rounded_full()
                    .bg(theme::accent()),
            )
            .into_any_element(),
        AttentionState::None => slot.into_any_element(),
    }
}

#[derive(Clone, Copy)]
pub(crate) enum DirectChatDisplayStatus {
    Busy,
    Idle,
    Stopped,
    Crashed,
}

pub(crate) fn direct_chat_display_status(
    session: &DirectSessionEntry,
    activity: Option<&SessionActivityState>,
) -> DirectChatDisplayStatus {
    match session.status {
        SessionStatus::Stopped => DirectChatDisplayStatus::Stopped,
        SessionStatus::Crashed => DirectChatDisplayStatus::Crashed,
        SessionStatus::Running => match activity {
            Some(SessionActivityState::Idle) => DirectChatDisplayStatus::Idle,
            Some(SessionActivityState::Busy) | None => DirectChatDisplayStatus::Busy,
        },
    }
}

pub(crate) fn session_label(entry: &DirectSessionEntry) -> String {
    entry.title.clone().unwrap_or_else(|| {
        entry
            .handle
            .as_ref()
            .map(|handle| format!("@{handle}"))
            .unwrap_or_else(|| entry.display_name.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        prelude::*, size, Context, Render, ScrollHandle, TestAppContext, VisualTestContext, Window,
    };
    use runner_backend::events::AppEvent;

    struct SidebarScrollLayoutTest {
        scroll: ScrollHandle,
        block_wrapper_scroll: ScrollHandle,
        constrained_section_scroll: ScrollHandle,
        short_scroll: ScrollHandle,
    }

    impl Render for SidebarScrollLayoutTest {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .child(sidebar_scroll_column(
                    self.scroll.clone(),
                    "test-sidebar-node-scroll",
                    true,
                    false,
                    6,
                    12,
                ))
                .child(sidebar_scroll_column(
                    self.block_wrapper_scroll.clone(),
                    "test-block-wrapper-sidebar-node-scroll",
                    false,
                    false,
                    6,
                    12,
                ))
                .child(sidebar_scroll_column(
                    self.constrained_section_scroll.clone(),
                    "test-constrained-section-sidebar-node-scroll",
                    true,
                    true,
                    6,
                    12,
                ))
                .child(sidebar_scroll_column(
                    self.short_scroll.clone(),
                    "test-short-sidebar-node-scroll",
                    true,
                    false,
                    1,
                    2,
                ))
        }
    }

    fn sidebar_scroll_column(
        scroll_handle: ScrollHandle,
        id: &'static str,
        flex_frame: bool,
        constrain_sections: bool,
        project_count: usize,
        chat_count: usize,
    ) -> AnyElement {
        let projects = div()
            .flex()
            .flex_col()
            .children((0..project_count).map(|_| div().h(px(40.)).flex_none()));
        let mut chat_scope = div()
            .flex_1()
            .flex()
            .flex_col()
            .children((0..chat_count).map(|_| div().h(px(28.)).flex_none()));
        if constrain_sections {
            chat_scope = chat_scope.min_h(rems(28. / 16.));
        }
        let chats_selector = format!("TEST_CHATS_{id}");
        let mut chats = div()
            .debug_selector(move || chats_selector.clone())
            .mt(px(20.))
            .flex_1()
            .flex()
            .flex_col()
            .child(div().h(px(24.)).flex_none())
            .child(chat_scope);
        if constrain_sections {
            chats = chats.min_h(px(0.));
        }
        let scroll = sidebar_scroll_container(id, &scroll_handle)
            .child(projects)
            .child(chats);
        let wrapper = if flex_frame {
            sidebar_scroll_frame().child(scroll)
        } else {
            div().relative().min_h(px(0.)).flex_1().child(scroll)
        };

        div()
            .w(px(240.))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .min_h(px(0.))
                    .flex_1()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(wrapper),
            )
            .into_any_element()
    }

    fn appended_event(signal: &str) -> AppEvent {
        AppEvent {
            name: "event/appended",
            payload: serde_json::json!({ "event": { "type": signal } }),
        }
    }

    #[test]
    fn mission_refresh_filters_appended_signals() {
        for signal in [
            "mission_start",
            "mission_stopped",
            "ask_human",
            "human_question",
            "human_response",
            "runner_status",
        ] {
            assert_eq!(
                StoreRefreshKind::for_event(&appended_event(signal)),
                Some(StoreRefreshKind::Missions)
            );
        }
        assert_eq!(
            StoreRefreshKind::for_event(&appended_event("inbox_read")),
            None
        );
        assert_eq!(
            StoreRefreshKind::for_event(&AppEvent {
                name: "event/appended",
                payload: serde_json::json!({ "event": { "kind": "message" } }),
            }),
            None
        );
    }

    #[test]
    fn chat_tab_selection_only_appears_on_the_chat_route() {
        assert!(chat_tab_row_active(&AppRoute::Chat, Some("tab-1"), "tab-1"));
        assert!(!chat_tab_row_active(
            &AppRoute::Chat,
            Some("tab-2"),
            "tab-1"
        ));
        assert!(!chat_tab_row_active(
            &AppRoute::Mission("mission-1".into()),
            Some("tab-1"),
            "tab-1"
        ));
        assert!(!chat_tab_row_active(
            &AppRoute::Settings,
            Some("tab-1"),
            "tab-1"
        ));
    }

    #[test]
    fn sidebar_scroll_layout_reports_overflow_and_fills_short_lists() {
        let mut cx = TestAppContext::single();
        let scroll = ScrollHandle::new();
        let test_scroll = scroll.clone();
        let block_wrapper_scroll = ScrollHandle::new();
        let test_block_wrapper_scroll = block_wrapper_scroll.clone();
        let constrained_section_scroll = ScrollHandle::new();
        let test_constrained_section_scroll = constrained_section_scroll.clone();
        let short_scroll = ScrollHandle::new();
        let test_short_scroll = short_scroll.clone();
        let window = cx.add_window(move |_, _| SidebarScrollLayoutTest {
            scroll: test_scroll,
            block_wrapper_scroll: test_block_wrapper_scroll,
            constrained_section_scroll: test_constrained_section_scroll,
            short_scroll: test_short_scroll,
        });
        cx.run_until_parked();
        let mut window = VisualTestContext::from_window(window.into(), &cx);
        window.simulate_resize(size(px(960.), px(160.)));
        window.run_until_parked();

        let viewport = f32::from(scroll.bounds().size.height);
        let max_offset = f32::from(scroll.max_offset().height);
        let block_wrapper_viewport = f32::from(block_wrapper_scroll.bounds().size.height);
        let block_wrapper_max_offset = f32::from(block_wrapper_scroll.max_offset().height);
        let constrained_section_max_offset =
            f32::from(constrained_section_scroll.max_offset().height);
        let constrained_chats_height = f32::from(
            window
                .debug_bounds("TEST_CHATS_test-constrained-section-sidebar-node-scroll")
                .unwrap()
                .size
                .height,
        );
        let short_chats_height = f32::from(
            window
                .debug_bounds("TEST_CHATS_test-short-sidebar-node-scroll")
                .unwrap()
                .size
                .height,
        );
        assert_eq!(viewport, 160.);
        assert_eq!(max_offset, 460.);
        assert_eq!(block_wrapper_viewport, 620.);
        assert_eq!(block_wrapper_max_offset, 0.);
        assert_eq!(constrained_section_max_offset, 100.);
        assert_eq!(constrained_chats_height, 0.);
        assert_eq!(short_chats_height, 100.);
    }
}
