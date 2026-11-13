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
        let submenu_root = root.clone();
        let submenu_actions = actions.clone();
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
            .return_focus(return_focus)
            .on_submenu(Rc::new(move |index, bounds, window, cx| {
                if let Some(SidebarMenuAction::LinkToChat(mission_id)) =
                    submenu_actions.get(index).cloned()
                {
                    submenu_root.update(cx, |this, cx| {
                        this.open_link_picker(
                            mission_id,
                            None,
                            None,
                            super::link::PickerAnchor::Row(bounds),
                            window,
                            cx,
                        )
                    });
                }
            }));
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

    /// Whether the sidebar row of tab `tab_id` is pinned, or `None` when the
    /// tab has no row to pin.
    pub(crate) fn tab_pinned(&self, tab_id: &str, cx: &App) -> Option<bool> {
        if self.front_page.is_remote() {
            self.source_nodes(cx)
                .into_iter()
                .find(|node| node.id == tab_id)
                .map(|node| node.pinned_position.is_some())
        } else {
            self.app_store
                .read(cx)
                .nodes
                .iter()
                .find(|node| node.id == tab_id)
                .map(|node| node.pinned_position.is_some())
        }
    }

    pub(crate) fn toggle_tab_pin(
        &mut self,
        tab_id: String,
        pinned: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_sidebar_menu_action(
            SidebarMenuAction::TogglePin {
                node_id: tab_id,
                pinned,
            },
            window,
            cx,
        );
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
            multi_pane,
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
        let pinned = self.mission_menu_pinned(&node, cx);
        let mut entries = mission_menu_entries(
            &node.id,
            pinned,
            &summary.mission.id,
            summary.mission.title,
            summary.mission.linked_chat.as_ref(),
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

    /// A linked mission's Pin acts on its group, so its menu reads the group's pin.
    pub(super) fn mission_menu_pinned(&self, node: &NodeRow, cx: &App) -> bool {
        let rows = self.resolved_sidebar_rows(cx);
        super::link::LinkGroups::of(&rows)
            .head(&node.id)
            .and_then(|head| rows.iter().find(|row| row.node().id == head))
            .map_or(node.pinned_position.is_some(), |head| {
                head.node().pinned_position.is_some()
            })
    }

    /// A sidebar +: the launcher page in `scope`, where chats, terminals and
    /// missions start.
    pub(crate) fn open_launcher_in(
        &mut self,
        scope: ProjectScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let action = SidebarMenuAction::NewChat(scope);
        if self.menu_action_supported(&action, cx) {
            self.handle_sidebar_menu_action(action, window, cx);
        }
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
                SidebarMenuAction::DeleteProject(id) => {
                    if let Some(shell) = self.shell.upgrade() {
                        let id = id.clone();
                        shell.update(cx, |root, cx| {
                            root.remote_client.open_delete_project(id, cx);
                        });
                    }
                    return;
                }
                SidebarMenuAction::StopMission(id) | SidebarMenuAction::ResumeMission(id) => {
                    let id = id.clone();
                    let action = action.clone();
                    if let Some(shell) = self.shell.upgrade() {
                        shell.update(cx, |root, cx| {
                            let feature = match action {
                                SidebarMenuAction::StopMission(_) => "stopMission",
                                SidebarMenuAction::ResumeMission(_) => "resumeMission",
                                _ => "archive",
                            };
                            root.remote_client.remote_action(
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
                                        _ => unreachable!(),
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
                if let Some(shell) = self.shell.upgrade() {
                    window.defer(cx, move |window, cx| {
                        shell.update(cx, |shell, shell_cx| {
                            shell.reveal_started_scope(&scope, shell_cx);
                            shell.open_sidebar_chat_modal(scope, window, shell_cx)
                        });
                    });
                }
            }
            SidebarMenuAction::TogglePin { node_id, pinned } => {
                let Some(node) = self
                    .source_nodes(cx)
                    .into_iter()
                    .find(|node| node.id == node_id)
                else {
                    return;
                };
                let operation = move |client: crate::surfaces::front_page::FrontPageClient| {
                    client.pin(&node, !pinned)
                };
                if self.front_page.is_remote() {
                    if let Some(shell) = self.shell.upgrade() {
                        shell.update(cx, |root, cx| {
                            root.remote_client.remote_action(
                                format!("pin:{node_id}"),
                                &["pin"],
                                move |client| {
                                    operation(client)?;
                                    Ok(Default::default())
                                },
                                cx,
                            )
                        });
                    }
                } else {
                    match operation(self.front_page.clone()) {
                        Ok(()) => self.refresh_store(StoreRefreshKind::All, cx),
                        Err(error) => self.report_error(error.to_string(), cx),
                    }
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
                                let layout = if shell.remote_client.is_active() {
                                    shell.remote_client.tab_layout(node_id)
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
            SidebarMenuAction::LinkToChat(mission_id) => {
                self.open_link_picker(
                    mission_id,
                    None,
                    None,
                    super::link::PickerAnchor::Pointer,
                    window,
                    cx,
                );
            }
            SidebarMenuAction::UnlinkMission(mission_id) => self.unlink_mission(mission_id, cx),
            SidebarMenuAction::ArchiveMission(mission_id) => {
                if self.archiving_missions.contains(&mission_id) {
                    return;
                }
                let archive_id = mission_id.clone();
                let operation = move |client: crate::surfaces::front_page::FrontPageClient| {
                    client.archive_mission_state(archive_id).map(drop)
                };
                if self.front_page.is_remote() {
                    if let Some(shell) = self.shell.upgrade() {
                        shell.update(cx, |root, cx| {
                            root.remote_client.remote_action(
                                format!("mission:{mission_id}"),
                                &["archive"],
                                move |client| {
                                    operation(client)?;
                                    Ok(Default::default())
                                },
                                cx,
                            )
                        });
                    }
                    return;
                }
                self.set_mission_archiving(mission_id.clone(), true, cx);
                let core = self.front_page.clone();
                let archive_task = cx.background_spawn(async move {
                    operation(core).map_err(|error| error.to_string())
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

/// One ordered list for a tab row, matching the pane title menu: Rename… ·
/// Fork chat · Pin, then the destructive end action after a separator.
#[allow(clippy::too_many_arguments)]
pub(super) fn tab_menu_entries(
    node_id: &str,
    pinned: bool,
    split: bool,
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
    let mut entries = vec![(
        UiMenuItem::new(if split {
            crate::i18n::message("ui.rename_tab_action")
        } else {
            crate::i18n::message("ui.rename_chat_action")
        })
        .icon("pencil.svg"),
        SidebarMenuAction::Rename(SidebarRenameTarget::Tab {
            node_id: node_id.to_owned(),
            original,
            session_id: rename_session_id,
        }),
    )];
    if let Some(target) = fork_target {
        let mut item = UiMenuItem::new(crate::i18n::message("ui.fork_chat_action"))
            .icon("git-fork.svg")
            .disabled(fork_pending || target.disabled);
        if let Some(reason) = target.reason {
            item = item.tooltip(reason);
        }
        entries.push((item, SidebarMenuAction::ForkChat(target.session_id)));
    }
    entries.push((
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
    ));
    if !chat_session_ids.is_empty() {
        let mut item = UiMenuItem::new(if split {
            crate::i18n::message("ui.archive_all")
        } else {
            crate::i18n::message("ui.archive")
        })
        .icon("archive.svg")
        .separator_before(true)
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
            .separator_before(true)
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
    linked: Option<&runner_core::protocol::LinkedChat>,
) -> Vec<(UiMenuItem, SidebarMenuAction)> {
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
            UiMenuItem::new(crate::i18n::message("ui.rename")).icon("pencil.svg"),
            SidebarMenuAction::Rename(SidebarRenameTarget::Mission {
                mission_id: mission_id.to_owned(),
                original,
            }),
        ),
        (
            UiMenuItem::new(crate::i18n::message("ui.link_to_chat"))
                .icon("link.svg")
                .submenu(),
            SidebarMenuAction::LinkToChat(mission_id.to_owned()),
        ),
    ];
    if let Some(chat) = linked {
        entries.push((
            UiMenuItem::new(crate::i18n::messagef(
                "format.unlink_from_chat",
                &[("chat", chat.title.clone())],
            ))
            .icon("link-2-off.svg"),
            SidebarMenuAction::UnlinkMission(mission_id.to_owned()),
        ));
    }
    entries.push((
        UiMenuItem::new(crate::i18n::message("ui.archive"))
            .icon("archive.svg")
            .destructive(true),
        SidebarMenuAction::ArchiveMission(mission_id.to_owned()),
    ));
    entries
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
