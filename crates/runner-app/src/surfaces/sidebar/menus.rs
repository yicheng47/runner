use super::elements::sidebar_fork_menu_target;
use runner_backend::model::Runtime;

use super::*;
use crate::*;
use runner_backend::ops::mission::MissionSummary;
use runner_backend::repo::node::NodeRow;

impl Sidebar {
    pub(super) fn open_sidebar_context_menu(
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

    pub(super) fn open_tab_menu(
        &mut self,
        node: NodeRow,
        layout: PaneLayout,
        members: Vec<DirectSessionEntry>,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let multi_pane = layout.root.leaves().len() > 1;
        let archive_all = multi_pane || !layout.drawer_shells().is_empty();
        let terminal_session_id = (!multi_pane)
            .then(|| {
                members
                    .iter()
                    .find(|member| Runtime::parse(&member.agent_runtime) == Some(Runtime::Shell))
                    .map(|member| member.session_id.clone())
            })
            .flatten();
        let rename_session = (!multi_pane).then(|| members.first()).flatten();
        let rename_original = rename_session.map_or_else(
            || layout.name.clone().unwrap_or_default(),
            |member| member.title.clone().unwrap_or_default(),
        );
        let rename_session_id = rename_session.map(|member| member.session_id.clone());
        let tab_session_ids = layout.all_session_ids();
        let fork_target = sidebar_fork_menu_target(&layout, &members);
        let fork_pending = fork_target.as_ref().is_some_and(|target| {
            self.shell.upgrade().is_some_and(|shell| {
                let shell = shell.read(cx);
                shell.fork_confirm.as_ref().is_some_and(|confirm| {
                    confirm.pending && confirm.session_id == target.session_id
                }) || super::chat::fork_in_progress(&shell.forking_sessions, &target.session_id)
            })
        });
        let entries = tab_menu_entries(
            &node.id,
            node.pinned_position.is_some(),
            rename_original,
            rename_session_id,
            archive_all,
            fork_target,
            fork_pending,
            members
                .into_iter()
                .filter(|member| Runtime::parse(&member.agent_runtime) != Some(Runtime::Shell))
                .map(|member| member.session_id)
                .collect(),
            tab_session_ids,
            terminal_session_id,
        );
        self.open_sidebar_context_menu(position, 160., entries, window, cx);
    }

    pub(super) fn open_mission_menu(
        &mut self,
        node: NodeRow,
        summary: MissionSummary,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entries = mission_menu_entries(
            &node.id,
            node.pinned_position.is_some(),
            &summary.mission.id,
            summary.mission.title,
        );
        self.open_sidebar_context_menu(position, 160., entries, window, cx);
    }

    pub(super) fn open_project_create_menu(
        &mut self,
        project_id: String,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_sidebar_context_menu(
            position,
            160.,
            project_create_menu_entries(&project_id),
            window,
            cx,
        );
    }

    pub(super) fn open_project_menu(
        &mut self,
        project: runner_backend::repo::project::ProjectRow,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entries = project_menu_entries(project.id, project.name);
        self.open_sidebar_context_menu(position, 200., entries, window, cx);
    }

    pub(super) fn handle_sidebar_menu_action(
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
            SidebarMenuAction::NewTerminal(project_id) => {
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
                            shell.new_terminal_tab(project_id, window, shell_cx)
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
            SidebarMenuAction::TogglePin { node_id, pinned } => {
                match runner_backend::ops::node::node_set_pinned(self.core(cx), node_id, !pinned) {
                    Ok(_) => self.refresh_store(StoreRefreshKind::All, cx),
                    Err(error) => self.report_error(error.to_string(), cx),
                }
            }
            SidebarMenuAction::Rename(target) => {
                let (value, placeholder) = match &target {
                    SidebarRenameTarget::Tab {
                        original, node_id, ..
                    } => {
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
            SidebarMenuAction::ForkChat(session_id) => {
                if let Some(shell) = self.shell.upgrade() {
                    window.defer(cx, move |window, cx| {
                        shell.update(cx, |shell, shell_cx| {
                            shell.fork_chat(&session_id, window, shell_cx)
                        });
                    });
                }
            }
            SidebarMenuAction::ArchiveTab {
                tab_id,
                session_ids,
            } => {
                if let Some(shell) = self.shell.upgrade() {
                    window.defer(cx, move |window, cx| {
                        shell.update(cx, |shell, shell_cx| {
                            shell.request_archive_all(
                                Some(&tab_id),
                                session_ids,
                                ArchiveAllSource::Sidebar,
                                window,
                                shell_cx,
                            )
                        });
                    });
                }
            }
            SidebarMenuAction::CloseTerminalTab { tab_id, session_id } => {
                if let Some(shell) = self.shell.upgrade() {
                    window.defer(cx, move |window, cx| {
                        shell.update(cx, |shell, shell_cx| {
                            shell.request_close_terminal_tab(&tab_id, &session_id, window, shell_cx)
                        });
                    });
                }
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
}

#[allow(clippy::too_many_arguments)]
pub(super) fn tab_menu_entries(
    node_id: &str,
    pinned: bool,
    original: String,
    rename_session_id: Option<String>,
    archive_all: bool,
    fork_target: Option<SidebarForkMenuTarget>,
    fork_pending: bool,
    chat_session_ids: Vec<String>,
    tab_session_ids: Vec<String>,
    terminal_session_id: Option<String>,
) -> Vec<(UiMenuItem, SidebarMenuAction)> {
    let mut entries = vec![
        (
            UiMenuItem::new(if pinned { "Unpin" } else { "Pin" }).icon(if pinned {
                "pin-off.svg"
            } else {
                "pin.svg"
            }),
            SidebarMenuAction::TogglePin {
                node_id: node_id.to_owned(),
                pinned,
            },
        ),
        (
            UiMenuItem::new("Rename tab").icon("pencil.svg"),
            SidebarMenuAction::Rename(SidebarRenameTarget::Tab {
                node_id: node_id.to_owned(),
                original,
                session_id: rename_session_id,
            }),
        ),
    ];
    if let Some(target) = fork_target {
        let mut item = UiMenuItem::new("Fork chat")
            .icon("git-fork.svg")
            .disabled(fork_pending || target.disabled);
        if let Some(reason) = target.description {
            item = item.description(reason);
        }
        entries.push((item, SidebarMenuAction::ForkChat(target.session_id)));
    }
    if !chat_session_ids.is_empty() {
        entries.push((
            UiMenuItem::new("Archive")
                .icon("archive.svg")
                .destructive(true),
            SidebarMenuAction::ArchiveTab {
                tab_id: node_id.to_owned(),
                session_ids: if archive_all {
                    tab_session_ids
                } else {
                    chat_session_ids
                },
            },
        ));
    } else if let Some(session_id) = terminal_session_id {
        entries.push((
            UiMenuItem::new("Close terminal")
                .icon("close.svg")
                .destructive(true),
            SidebarMenuAction::CloseTerminalTab {
                tab_id: node_id.to_owned(),
                session_id,
            },
        ));
    }
    entries
}

pub(super) fn sidebar_tab_icon(pane_count: usize, single_runtime: Option<&str>) -> ChatIcon {
    if pane_count > 1 {
        ChatIcon::split()
    } else {
        ChatIcon::for_runtime(single_runtime.unwrap_or_default())
    }
}

pub(super) fn mission_menu_entries(
    node_id: &str,
    pinned: bool,
    mission_id: &str,
    original: String,
) -> Vec<(UiMenuItem, SidebarMenuAction)> {
    vec![
        (
            UiMenuItem::new(if pinned { "Unpin" } else { "Pin" }).icon(if pinned {
                "pin-off.svg"
            } else {
                "pin.svg"
            }),
            SidebarMenuAction::TogglePin {
                node_id: node_id.to_owned(),
                pinned,
            },
        ),
        (
            UiMenuItem::new("Rename").icon("pencil.svg"),
            SidebarMenuAction::Rename(SidebarRenameTarget::Mission {
                mission_id: mission_id.to_owned(),
                original,
            }),
        ),
        (
            UiMenuItem::new("Archive")
                .icon("archive.svg")
                .destructive(true),
            SidebarMenuAction::ArchiveMission(mission_id.to_owned()),
        ),
    ]
}

pub(super) fn project_create_menu_entries(
    project_id: &str,
) -> Vec<(UiMenuItem, SidebarMenuAction)> {
    vec![
        (
            UiMenuItem::new("New chat").icon("message-square-plus.svg"),
            SidebarMenuAction::NewChat(Some(project_id.to_owned())),
        ),
        (
            UiMenuItem::new("New terminal").icon("square-terminal.svg"),
            SidebarMenuAction::NewTerminal(Some(project_id.to_owned())),
        ),
        (
            UiMenuItem::new("New mission").icon("flag.svg"),
            SidebarMenuAction::NewMission(Some(project_id.to_owned())),
        ),
    ]
}

pub(super) fn sidebar_create_menu_entries() -> Vec<(UiMenuItem, SidebarMenuAction)> {
    vec![
        (
            UiMenuItem::new("New chat").icon("message-square-plus.svg"),
            SidebarMenuAction::NewChat(None),
        ),
        (
            UiMenuItem::new("New terminal").icon("square-terminal.svg"),
            SidebarMenuAction::NewTerminal(None),
        ),
        (
            UiMenuItem::new("New mission").icon("flag.svg"),
            SidebarMenuAction::NewMission(None),
        ),
    ]
}

pub(super) fn project_menu_entries(
    project_id: String,
    project_name: String,
) -> Vec<(UiMenuItem, SidebarMenuAction)> {
    vec![
        (
            UiMenuItem::new("Rename project").icon("pencil.svg"),
            SidebarMenuAction::Rename(SidebarRenameTarget::Project {
                project_id: project_id.clone(),
                original: project_name,
            }),
        ),
        (
            UiMenuItem::new("Delete project")
                .icon("trash.svg")
                .separator_before(true)
                .destructive(true),
            SidebarMenuAction::DeleteProject(project_id),
        ),
    ]
}
