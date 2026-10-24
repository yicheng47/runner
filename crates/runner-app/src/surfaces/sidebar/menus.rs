use super::elements::sidebar_fork_menu_target;
use runner_core::protocol::model::Runtime;

use super::*;
use crate::*;

/// Where a sidebar menu opens: at the pointer, as a right-click menu does,
/// or under the row button that opened it.
#[derive(Clone, Copy)]
pub(super) enum MenuOrigin {
    Pointer(gpui::Point<gpui::Pixels>),
    Button(gpui::Bounds<gpui::Pixels>),
}

impl Sidebar {
    pub(super) fn open_sidebar_context_menu(
        &mut self,
        position: gpui::Point<gpui::Pixels>,
        entries: Vec<(UiMenuItem, SidebarMenuAction)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_sidebar_menu(MenuOrigin::Pointer(position), entries, window, cx);
    }

    /// Opens a menu at `origin`. Pressing the button a menu is open under
    /// closes it instead.
    fn open_sidebar_menu(
        &mut self,
        origin: MenuOrigin,
        entries: Vec<(UiMenuItem, SidebarMenuAction)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entries = entries
            .into_iter()
            .filter(|(_, action)| self.menu_action_supported(action, cx))
            .collect::<Vec<_>>();
        if entries.is_empty() {
            return;
        }

        let (position, anchor) = match origin {
            MenuOrigin::Pointer(position) => (position, None),
            MenuOrigin::Button(anchor) => (anchor.bottom_left(), Some(anchor)),
        };
        if anchor.is_some()
            && self
                .context_menu
                .as_ref()
                .is_some_and(|menu| menu.read(cx).anchor() == anchor)
        {
            if let Some(menu) = self.context_menu.take() {
                menu.update(cx, |menu, cx| menu.close(window, cx));
            }
            self.schedule_shell_notify(cx);
            cx.notify();
            return;
        }
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
        let return_focus = window.focused(cx);
        let menu = cx.new(move |menu_cx| {
            let action_root = root.clone();
            let menu = ContextMenu::new(
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
            .return_focus(return_focus);
            match anchor {
                Some(anchor) => menu.anchored_to(anchor),
                None => menu,
            }
        });
        let focus = menu.read(cx).focus_handle();
        self.context_menu = Some(menu);
        focus.focus(window, cx);
        self.schedule_shell_notify(cx);
        cx.notify();
    }

    pub(super) fn open_tab_menu(
        &mut self,
        node: NodeRow,
        layout: PaneLayout,
        members: Vec<SessionDisplay>,
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
                    .find(|member| {
                        Runtime::parse(&member.agent_runtime).is_some_and(Runtime::is_shell)
                    })
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
                .filter(|member| {
                    !Runtime::parse(&member.agent_runtime).is_some_and(Runtime::is_shell)
                })
                .map(|member| member.session_id)
                .collect(),
            tab_session_ids,
            terminal_session_id,
            &self.settings(cx).keymap_overrides,
        );
        self.open_sidebar_context_menu(position, entries, window, cx);
    }

    pub(super) fn open_mission_menu(
        &mut self,
        node: NodeRow,
        summary: MissionSummary,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut entries = mission_menu_entries(
            &node.id,
            node.pinned_position.is_some(),
            &summary.mission.id,
            summary.mission.title,
        );
        if self.front_page.is_remote() {
            entries.insert(
                0,
                if summary.mission.running {
                    (
                        UiMenuItem::new(crate::i18n::message("ui.stop")).icon("square.svg"),
                        SidebarMenuAction::StopMission(summary.mission.id.clone()),
                    )
                } else {
                    (
                        UiMenuItem::new(crate::i18n::message("ui.resume")).icon("play.svg"),
                        SidebarMenuAction::ResumeMission(summary.mission.id.clone()),
                    )
                },
            );
        }
        self.open_sidebar_context_menu(position, entries, window, cx);
    }

    pub(super) fn open_project_create_menu(
        &mut self,
        project_id: String,
        anchor: gpui::Bounds<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_sidebar_menu(
            MenuOrigin::Button(anchor),
            project_create_menu_entries(&project_id, &self.settings(cx).keymap_overrides),
            window,
            cx,
        );
    }

    pub(super) fn open_project_menu(
        &mut self,
        project: crate::surfaces::front_page::model::ProjectDisplay,
        origin: MenuOrigin,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entries = project_menu_entries(project.id, project.name);
        self.open_sidebar_menu(origin, entries, window, cx);
    }

    pub(super) fn handle_sidebar_menu_action(
        &mut self,
        action: SidebarMenuAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.menu_action_supported(&action, cx) {
            return;
        }
        if self.front_page.is_remote() {
            match &action {
                SidebarMenuAction::TogglePin { node_id, pinned } => {
                    let Some(node) = self
                        .source_nodes(cx)
                        .into_iter()
                        .find(|node| &node.id == node_id)
                    else {
                        return;
                    };
                    if let Some(shell) = self.shell.upgrade() {
                        let pinned = !pinned;
                        shell.update(cx, |root, cx| {
                            root.remote_action(
                                format!("pin:{}", node.id),
                                &["pin"],
                                move |client| {
                                    client.pin(&node, pinned)?;
                                    Ok(Default::default())
                                },
                                cx,
                            )
                        });
                    }
                    return;
                }
                SidebarMenuAction::DeleteProject(id) => {
                    if let Some(shell) = self.shell.upgrade() {
                        let id = id.clone();
                        shell.update(cx, |root, cx| {
                            if let Some(remote) = root.remote_window.as_mut() {
                                remote.delete_project = Some(id);
                                remote.delete_project_force = false;
                                remote.delete_project_error = None;
                                cx.notify();
                            }
                        });
                    }
                    return;
                }
                SidebarMenuAction::ArchiveMission(id)
                | SidebarMenuAction::StopMission(id)
                | SidebarMenuAction::ResumeMission(id) => {
                    let id = id.clone();
                    let action = action.clone();
                    if let Some(shell) = self.shell.upgrade() {
                        shell.update(cx, |root, cx| {
                            let feature = match action {
                                SidebarMenuAction::StopMission(_) => "stopMission",
                                SidebarMenuAction::ResumeMission(_) => "resumeMission",
                                _ => "archive",
                            };
                            root.remote_action(
                                format!("mission:{id}"),
                                &[feature],
                                move |client| {
                                    let mut done =
                                        crate::surfaces::front_page::actions::Completion::default();
                                    match action {
                                        SidebarMenuAction::StopMission(_) => {
                                            client.stop_mission(id)?
                                        }
                                        SidebarMenuAction::ResumeMission(_) => {
                                            done.reopened_sessions = client
                                                .resume_mission(id, (INITIAL_COLS, INITIAL_ROWS))?
                                        }
                                        _ => client.archive(
                                            runner_core::companion::ActionTarget {
                                                kind: runner_core::companion::ActionKind::Mission,
                                                id,
                                            },
                                        )?,
                                    }
                                    Ok(done)
                                },
                                cx,
                            );
                        });
                    }
                    return;
                }
                _ => {}
            }
        }

        match action {
            SidebarMenuAction::NewChat(scope) => {
                self.set_active_project(scope.project_id().map(str::to_owned), cx);
                if let Some(project_id) = scope.project_id() {
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
                            shell.open_sidebar_chat_modal(scope, window, shell_cx)
                        });
                    });
                }
            }
            SidebarMenuAction::NewTerminal(scope) => {
                self.set_active_project(scope.project_id().map(str::to_owned), cx);
                if let Some(project_id) = scope.project_id() {
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
                            shell.new_terminal_tab(scope, window, shell_cx)
                        });
                    });
                }
            }
            SidebarMenuAction::NewMission(scope) => {
                self.set_active_project(scope.project_id().map(str::to_owned), cx);
                let client = self.front_page.clone();
                if let Some(shell) = self.shell.upgrade() {
                    window.defer(cx, move |window, cx| {
                        shell.update(cx, |shell, shell_cx| {
                            if !client.valid()
                                || shell.front_page_client(shell_cx).target != client.target
                            {
                                return;
                            }
                            shell.open_start_mission_modal(None, scope, window, shell_cx)
                        });
                    });
                }
            }
            SidebarMenuAction::TogglePin { node_id, pinned } => {
                match self.core(cx).node_set_pinned(node_id, !pinned) {
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
                                let layout = if let Some(remote) = &shell.remote_window {
                                    remote.layouts.get(node_id)
                                } else {
                                    shell
                                        .tabs
                                        .tabs()
                                        .iter()
                                        .find(|layout| layout.id == *node_id)
                                };
                                layout.map(|layout| shell.tab_label_source(layout, cx))
                            })
                            .unwrap_or_else(|| crate::i18n::message("ui.chat_tab"));
                        (original.clone(), placeholder)
                    }
                    SidebarRenameTarget::Project { original, .. }
                    | SidebarRenameTarget::Mission { original, .. } => {
                        (original.clone(), crate::i18n::Text::from(original.clone()))
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
                if self.archiving_missions.contains(&mission_id) {
                    return;
                }
                self.set_mission_archiving(mission_id.clone(), true, cx);
                let core = self.core(cx).clone();
                let archive_id = mission_id.clone();
                let archive_task = cx.background_spawn(async move {
                    core.mission_archive_impl(archive_id)
                        .map(drop)
                        .map_err(|error| error.to_string())
                });
                cx.spawn_in(window, async move |weak, cx| {
                    let result = archive_task.await;
                    let _ = weak.update_in(cx, |this, window, cx| {
                        this.set_mission_archiving(mission_id.clone(), false, cx);
                        match result {
                            Ok(()) => {
                                if let Some(shell) = this.shell.upgrade() {
                                    window.defer(cx, move |window, cx| {
                                        shell.update(cx, |shell, shell_cx| {
                                            shell.leave_archived_mission(
                                                &mission_id,
                                                Some(false),
                                                true,
                                                window,
                                                shell_cx,
                                            );
                                        });
                                    });
                                }
                            }
                            Err(error) => this.report_error(error, cx),
                        }
                        this.refresh_store(StoreRefreshKind::All, cx);
                    });
                })
                .detach();
            }
            SidebarMenuAction::StopMission(_) | SidebarMenuAction::ResumeMission(_) => {}
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
    overrides: &keymap::KeymapOverrides,
) -> Vec<(UiMenuItem, SidebarMenuAction)> {
    let close_shortcut = (!archive_all && tab_session_ids.len() == 1)
        .then(|| keymap::effective_binding("close-pane", overrides))
        .flatten()
        .map(|combo| keymap::format_combo(&combo).into());
    let mut entries = vec![
        (
            UiMenuItem::new(if pinned {
                crate::i18n::message("ui.unpin")
            } else {
                crate::i18n::message("ui.pin")
            })
            .icon(if pinned { "pin-off.svg" } else { "pin.svg" }),
            SidebarMenuAction::TogglePin {
                node_id: node_id.to_owned(),
                pinned,
            },
        ),
        (
            UiMenuItem::new(crate::i18n::message("ui.rename_tab")).icon("pencil.svg"),
            SidebarMenuAction::Rename(SidebarRenameTarget::Tab {
                node_id: node_id.to_owned(),
                original,
                session_id: rename_session_id,
            }),
        ),
    ];
    if let Some(target) = fork_target {
        let mut item = UiMenuItem::new(crate::i18n::message("ui.fork_chat_action"))
            .icon("git-fork.svg")
            .disabled(fork_pending || target.disabled);
        if let Some(reason) = target.reason {
            item = item.tooltip(reason);
        }
        entries.push((item, SidebarMenuAction::ForkChat(target.session_id)));
    }
    if !chat_session_ids.is_empty() {
        let mut item = UiMenuItem::new(crate::i18n::message("ui.archive"))
            .icon("archive.svg")
            .destructive(true);
        item.shortcut = close_shortcut;
        entries.push((
            item,
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
        let mut item = UiMenuItem::new(crate::i18n::message("ui.close_terminal"))
            .icon("close.svg")
            .destructive(true);
        item.shortcut = close_shortcut;
        entries.push((
            item,
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
            UiMenuItem::new(if pinned {
                crate::i18n::message("ui.unpin")
            } else {
                crate::i18n::message("ui.pin")
            })
            .icon(if pinned { "pin-off.svg" } else { "pin.svg" }),
            SidebarMenuAction::TogglePin {
                node_id: node_id.to_owned(),
                pinned,
            },
        ),
        (
            UiMenuItem::new(crate::i18n::message("ui.rename")).icon("pencil.svg"),
            SidebarMenuAction::Rename(SidebarRenameTarget::Mission {
                mission_id: mission_id.to_owned(),
                original,
            }),
        ),
        (
            UiMenuItem::new(crate::i18n::message("ui.archive"))
                .icon("archive.svg")
                .destructive(true),
            SidebarMenuAction::ArchiveMission(mission_id.to_owned()),
        ),
    ]
}

pub(super) fn project_create_menu_entries(
    project_id: &str,
    overrides: &keymap::KeymapOverrides,
) -> Vec<(UiMenuItem, SidebarMenuAction)> {
    create_menu_entries(ProjectScope::Project(project_id.to_owned()), overrides)
}

pub(super) fn sidebar_create_menu_entries(
    overrides: &keymap::KeymapOverrides,
) -> Vec<(UiMenuItem, SidebarMenuAction)> {
    create_menu_entries(ProjectScope::Root, overrides)
}

fn create_menu_entries(
    scope: ProjectScope,
    overrides: &keymap::KeymapOverrides,
) -> Vec<(UiMenuItem, SidebarMenuAction)> {
    [
        (
            "new-chat",
            crate::i18n::message("ui.new_chat"),
            "message-square-plus.svg",
            SidebarMenuAction::NewChat(scope.clone()),
        ),
        (
            "new-mission",
            crate::i18n::message("ui.new_mission"),
            "flag.svg",
            SidebarMenuAction::NewMission(scope.clone()),
        ),
        (
            "new-terminal",
            crate::i18n::message("ui.new_terminal"),
            "square-terminal.svg",
            SidebarMenuAction::NewTerminal(scope),
        ),
    ]
    .into_iter()
    .map(|(id, label, icon, action)| {
        let mut item = UiMenuItem::new(label).icon(icon);
        if let Some(combo) = keymap::effective_binding(id, overrides) {
            item = item.shortcut(keymap::format_combo(&combo));
        }
        (item, action)
    })
    .collect()
}

pub(super) fn project_menu_entries(
    project_id: String,
    project_name: String,
) -> Vec<(UiMenuItem, SidebarMenuAction)> {
    vec![
        (
            UiMenuItem::new(crate::i18n::message("ui.rename_project")).icon("pencil.svg"),
            SidebarMenuAction::Rename(SidebarRenameTarget::Project {
                project_id: project_id.clone(),
                original: project_name,
            }),
        ),
        (
            UiMenuItem::new(crate::i18n::message("ui.delete_project"))
                .icon("trash.svg")
                .separator_before(true)
                .destructive(true),
            SidebarMenuAction::DeleteProject(project_id),
        ),
    ]
}
