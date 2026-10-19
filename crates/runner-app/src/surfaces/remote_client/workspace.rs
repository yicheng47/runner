use super::*;
use runner_app::ui::button::spinner;
use runner_core::companion::{Overview, SidebarNode, SidebarNodeKind, Status};
use runner_core::remote::RemoteTerminal;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum Item {
    Roles,
    Crews,
    Tab(String),
    Mission(String),
}
pub(super) struct RemotePane {
    _lifetime: transport::AttachmentLifetime,
    pub(super) terminal: Arc<TerminalMirror>,
    _view: TerminalView,
    interaction: Entity<TerminalInteraction>,
    pub(super) input: Entity<TerminalInput>,
    focus: FocusHandle,
    scrollbar: Entity<Scrollbar>,
    _subscription: Subscription,
    control_failed: bool,
}
pub(crate) struct RemoteWindow {
    pub(crate) computer: Computer,
    pub(crate) front_page: crate::surfaces::front_page::FrontPageClient,
    pub(crate) sidebar: Entity<Sidebar>,
    pub(crate) mission_workspace: Entity<MissionWorkspace>,
    pub(crate) sidebar_collapsed: bool,
    pub(crate) sidebar_width: f32,
    _follow: Option<model::Follow>,
    pub(crate) active: Option<Item>,
    pub(crate) layouts: HashMap<String, PaneLayout>,
    pub(crate) overview_revision: u64,
    pub(crate) resize_draft: Option<PaneLayout>,
    pub(crate) resizing_split: Option<String>,
    pub(crate) drawer_resizing: bool,
    pub(crate) pane_bounds: HashMap<PaneKey, gpui::Size<Pixels>>,
    pub(crate) layout_hints: HashMap<String, PaneLayout>,
    pub(crate) actions_pending: HashSet<String>,
    pub(crate) action_error: Option<String>,
    pub(crate) shell_placement: Arc<Mutex<Option<(String, PaneLayout)>>>,
    pub(crate) catalog: Option<runner_core::companion::Catalog>,
    pub(crate) library_catalog: Option<runner_core::companion::Catalog>,
    pub(crate) library_request: u64,
    pub(crate) library_target: Option<crate::surfaces::front_page::FrontPageTarget>,
    pub(crate) role_surfaces: crate::surfaces::roles::RoleSurfaces,
    pub(crate) crew_surfaces: crate::surfaces::crews::CrewSurfaces,
    catalog_target: Option<crate::surfaces::front_page::FrontPageTarget>,
    pub(crate) project_picker:
        Option<Entity<crate::surfaces::front_page::folder_picker::FolderPicker>>,
    pub(crate) project_picker_subscription: Option<Subscription>,
    pub(crate) delete_project: Option<String>,
    pub(crate) delete_project_busy: bool,
    pub(crate) delete_project_force: bool,
    pub(crate) delete_project_error: Option<String>,
    pub(crate) navigation_generation: u64,
    pub(crate) activate_session: Option<String>,
    pub(crate) activate_placement: Option<(String, Option<String>)>,
    pub(crate) reopen_sessions: HashSet<String>,
    pub(crate) activate_tab: Option<String>,
    pub(crate) activate_mission: Option<String>,
    pub(crate) form_generation: u64,
    pub(super) terminals: HashMap<String, RemotePane>,
    recovery: HashSet<String>,
    terminal_epoch: u64,
    pair_epoch: u64,
    attachment_generation: u64,
    pending: HashMap<String, Arc<()>>,
    sizes: HashMap<String, (u16, u16)>,
    errors: HashSet<String>,
    connected: Arc<AtomicBool>,
    pub(super) connection: Connection,
    epoch: u64,
    wake: futures::channel::mpsc::UnboundedSender<String>,
}
impl Drop for RemoteWindow {
    fn drop(&mut self) {
        self.connected.store(false, Ordering::Release);
    }
}
impl RemoteWindow {
    fn visible_sessions(&self, _overview: &Overview, cx: &App) -> Vec<String> {
        match self.active.as_ref() {
            Some(Item::Mission(_)) => self.mission_workspace.read(cx).remote_visible_sessions(),
            Some(Item::Tab(id)) => self
                .layouts
                .get(id)
                .map(|layout| {
                    let mut ids = layout.session_ids();
                    if layout.drawer_open() {
                        if let Some(id) = layout.active_drawer_shell() {
                            ids.push(id.into());
                        }
                    }
                    ids
                })
                .unwrap_or_default(),
            None | Some(Item::Roles | Item::Crews) => Vec::new(),
        }
    }
}

impl NativeRoot {
    pub(crate) fn report_remote_input_error(
        &mut self,
        address: &str,
        message: String,
        cx: &mut Context<Self>,
    ) {
        let Some((computer, session)) = runner_core::remote::terminal_target(address) else {
            return;
        };
        let clients = state(&self.app_store, cx);
        let Some(remote) = self.remote_window.as_mut().filter(|remote| {
            remote.computer.id == computer && remote.terminals.contains_key(session)
        }) else {
            return;
        };
        let visible = clients
            .read(cx)
            .overviews
            .get(computer)
            .is_some_and(|overview| {
                remote
                    .visible_sessions(overview, cx)
                    .iter()
                    .any(|id| id == session)
            });
        if visible {
            remote.action_error = Some(message);
            cx.notify();
        }
    }
    pub(crate) fn remote_terminal_interactive(&self, id: &str, cx: &App) -> bool {
        self.remote_window.as_ref().is_some_and(|remote| {
            remote.connection == Connection::Connected
                && remote
                    .terminals
                    .get(id)
                    .is_some_and(|pane| pane.terminal.control().desktop())
                && self
                    .app_store
                    .read(cx)
                    .remote_clients
                    .read(cx)
                    .overviews
                    .get(&remote.computer.id)
                    .and_then(|overview| overview.sessions.get(id))
                    .is_some_and(|session| {
                        !matches!(
                            session.status.status.effective(),
                            Status::Stopped | Status::Error
                        )
                    })
        })
    }
    pub(crate) fn remote_terminal_size(&self, id: &str) -> Option<(u16, u16)> {
        let remote = self.remote_window.as_ref()?;
        remote
            .terminals
            .get(id)
            .map(|pane| pane.terminal.size())
            .or_else(|| remote.sizes.get(id).copied())
    }
    pub(crate) fn focus_remote_terminal(&self, id: &str, window: &mut Window, cx: &mut App) {
        if let Some(pane) = self
            .remote_window
            .as_ref()
            .and_then(|remote| remote.terminals.get(id))
        {
            pane.focus.focus(window, cx);
        }
    }
    pub(crate) fn visit_computer(
        &mut self,
        computer: Computer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.app_store.read(cx).remote_access_enabled() {
            return;
        }
        if let Some(remote) = self
            .remote_window
            .as_mut()
            .filter(|remote| remote.computer.id == computer.id)
        {
            remote.computer = computer.clone();
            self.client_ui.picker_open = false;
            if !matches!(
                remote.connection,
                Connection::Unreachable | Connection::Revoked | Connection::Forgotten
            ) {
                cx.notify();
                return;
            }
            remote.terminals.clear();
            remote.pending.clear();
            remote.recovery.clear();
            remote.attachment_generation = remote.attachment_generation.wrapping_add(1);
            remote.errors.clear();
            state(&self.app_store, cx)
                .read(cx)
                .send(Command::Retry(computer.id));
            return;
        }
        if let Some(remote) = &self.remote_window {
            remote
                .mission_workspace
                .update(cx, |workspace, cx| workspace.release_window(cx));
        }
        self.start_chat_modal = None;
        self.start_mission_modal = None;
        self.chat_rename_modal = None;
        self.pane_rename = None;
        self.fork_confirm = None;
        self.terminal_close_confirm = None;
        self.command_palette
            .update(cx, |palette, cx| palette.close_for_target_change(cx));
        self.client_ui.epoch += 1;
        let epoch = self.client_ui.epoch;
        let clients = state(&self.app_store, cx);
        let follow = clients.read(cx).follow(computer.id.clone());
        let (wake, mut wakes) = futures::channel::mpsc::unbounded();
        cx.spawn(async move |weak, cx| {
            while let Some(id) = wakes.next().await {
                if weak
                    .update(cx, |root, cx| {
                        if root
                            .remote_window
                            .as_ref()
                            .is_some_and(|remote| remote.epoch == epoch)
                        {
                            root.app_store
                                .read(cx)
                                .terminal_updates
                                .clone()
                                .update(cx, |_, cx| cx.emit(id));
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let pair_epoch = clients
            .read(cx)
            .pair_epochs
            .get(&computer.id)
            .copied()
            .unwrap_or_default();
        let access = clients.update(cx, |state, cx| state.front_page_access(&computer, cx));
        let front_page =
            crate::surfaces::front_page::FrontPageClient::remote(self.core(cx).clone(), access);
        let shell = cx.weak_entity();
        let sidebar =
            cx.new(|sidebar_cx| Sidebar::new(shell, self.app_store.clone(), None, sidebar_cx));
        sidebar.update(cx, |sidebar, cx| {
            sidebar.set_front_page_target(front_page.clone(), cx)
        });
        let shell = cx.weak_entity();
        let mission_workspace = cx.new(|workspace_cx| {
            MissionWorkspace::new_remote(
                self.window_label.clone(),
                shell,
                self.app_store.clone(),
                front_page.clone(),
                window,
                workspace_cx,
            )
        });
        self.remote_window = Some(RemoteWindow {
            mission_workspace,
            sidebar,
            sidebar_collapsed: false,
            sidebar_width: self.settings(cx).sidebar_width,
            front_page,
            computer,
            _follow: Some(follow),
            active: None,
            layouts: HashMap::new(),
            overview_revision: 0,
            resize_draft: None,
            resizing_split: None,
            drawer_resizing: false,
            pane_bounds: HashMap::new(),
            layout_hints: HashMap::new(),
            actions_pending: HashSet::new(),
            action_error: None,
            shell_placement: Arc::new(Mutex::new(None)),
            catalog: None,
            library_catalog: None,
            library_request: 0,
            library_target: None,
            role_surfaces: crate::surfaces::roles::RoleSurfaces::new_remote(
                cx.entity(),
                self.settings(cx).roles_page_size,
                epoch,
                cx,
            ),
            crew_surfaces: crate::surfaces::crews::CrewSurfaces::new_remote(
                cx.entity(),
                self.settings(cx).crews_page_size,
                epoch,
                cx,
            ),
            catalog_target: None,
            project_picker: None,
            project_picker_subscription: None,
            delete_project: None,
            delete_project_busy: false,
            delete_project_force: false,
            delete_project_error: None,
            navigation_generation: 0,
            activate_session: None,
            activate_placement: None,
            reopen_sessions: HashSet::new(),
            activate_tab: None,
            activate_mission: None,
            form_generation: 0,
            terminals: HashMap::new(),
            recovery: HashSet::new(),
            terminal_epoch: clients.read(cx).terminal_epoch,
            pair_epoch,
            attachment_generation: 0,
            pending: HashMap::new(),
            sizes: HashMap::new(),
            errors: HashSet::new(),
            connected: Arc::new(AtomicBool::new(false)),
            connection: Connection::Opening,
            epoch,
            wake,
        });
        self.client_ui.picker_open = false;
        self.mission_workspace
            .update(cx, |workspace, cx| workspace.release_window(cx));
        self.root_focus.focus(window, cx);
        self.report_current_subjects(cx);
        self.sync_remote_state(cx);
        cx.notify();
    }
    pub(crate) fn return_to_this_computer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_chat_modal = None;
        self.start_mission_modal = None;
        self.chat_rename_modal = None;
        self.pane_rename = None;
        self.fork_confirm = None;
        self.terminal_close_confirm = None;
        self.command_palette
            .update(cx, |palette, cx| palette.close_for_target_change(cx));
        if let Some(remote) = &self.remote_window {
            remote
                .mission_workspace
                .update(cx, |workspace, cx| workspace.release_window(cx));
        }
        self.remote_window = None;
        self.client_ui.picker_open = false;
        self.root_focus.focus(window, cx);
        self.sync_window_activation(window, cx);
        cx.notify();
    }
    pub(crate) fn sync_remote_state(&mut self, cx: &mut Context<Self>) {
        self.sync_computer_pair(cx);
        let clients = state(&self.app_store, cx);
        let Some(remote) = self.remote_window.as_mut() else {
            cx.notify();
            return;
        };
        if let Some(computer) = clients
            .read(cx)
            .computers
            .iter()
            .find(|computer| computer.id == remote.computer.id)
        {
            remote.computer = computer.clone();
        }
        let pair_epoch = clients
            .read(cx)
            .pair_epochs
            .get(&remote.computer.id)
            .copied()
            .unwrap_or_default();
        if remote.pair_epoch != pair_epoch {
            remote.terminals.clear();
            remote.pending.clear();
            remote.recovery.clear();
            remote.errors.clear();
            remote.pair_epoch = pair_epoch;
            remote.attachment_generation = remote.attachment_generation.wrapping_add(1);
        }
        let terminal_epoch = clients.read(cx).terminal_epoch;
        if remote.terminal_epoch != terminal_epoch {
            for (id, pane) in &remote.terminals {
                if let Some(size) = pane.interaction.read(cx).pane_size {
                    remote
                        .sizes
                        .insert(id.clone(), (size.cols.max(20), size.rows.max(8)));
                }
            }
            remote.recovery.extend(remote.terminals.keys().cloned());
            remote.recovery.extend(remote.pending.keys().cloned());
            remote.terminals.clear();
            remote.pending.clear();
            remote.errors.clear();
            remote.terminal_epoch = terminal_epoch;
            remote.attachment_generation = remote.attachment_generation.wrapping_add(1);
        }
        let connection = clients.read(cx).connection(&remote.computer.id);
        let connection = if !remote.computer.paired && connection == Connection::Connected {
            Connection::Revoked
        } else {
            connection
        };
        let trusted = clients.read(cx).computers.iter().any(|computer| {
            computer.id == remote.computer.id
                && computer.fingerprint == remote.computer.fingerprint
                && computer.paired
        }) && !matches!(connection, Connection::Revoked | Connection::Forgotten);
        if !trusted {
            remote._follow = None;
        } else if remote._follow.is_none() {
            remote._follow = Some(clients.read(cx).follow(remote.computer.id.clone()));
        }

        if connection == Connection::Forgotten && remote.connection != connection {
            remote.terminals.clear();
            remote.pending.clear();
            remote.recovery.clear();
            remote.errors.clear();
            remote.attachment_generation = remote.attachment_generation.wrapping_add(1);
        }
        let revision = clients
            .read(cx)
            .overview_revisions
            .get(&remote.computer.id)
            .copied()
            .unwrap_or_default();
        let pushed = revision != remote.overview_revision;
        remote.overview_revision = revision;
        if let Some(overview) = clients.read(cx).overviews.get(&remote.computer.id) {
            if pushed {
                remote.layout_hints.retain(|id, _| {
                    !remote.layouts.contains_key(id)
                        || overview.tabs.iter().any(|tab| &tab.id == id)
                });
            }
            remote
                .layouts
                .retain(|id, _| overview.tabs.iter().any(|tab| &tab.id == id));
            for tab in &overview.tabs {
                let layout = crate::surfaces::front_page::model::reconcile_tab(
                    tab,
                    remote.layouts.get(&tab.id),
                    remote.layout_hints.get(&tab.id),
                );
                if remote.layout_hints.get(&tab.id).is_some_and(|hint| {
                    crate::surfaces::front_page::model::matches_layout(hint, tab)
                        || (pushed
                            && remote.layouts.get(&tab.id).is_some_and(|old| {
                                !crate::surfaces::front_page::model::matches_layout(old, tab)
                            }))
                }) {
                    remote.layout_hints.remove(&tab.id);
                }
                remote.layouts.insert(tab.id.clone(), layout);
            }
        }
        if let Some(overview) = clients.read(cx).overviews.get(&remote.computer.id) {
            let live = remote
                .reopen_sessions
                .iter()
                .filter(|id| {
                    overview.sessions.get(*id).is_some_and(|session| {
                        !matches!(
                            session.status.status.effective(),
                            Status::Stopped | Status::Error
                        )
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
            for id in live {
                remote.reopen_sessions.remove(&id);
                remote.terminals.remove(&id);
                remote.pending.remove(&id);
                remote.recovery.remove(&id);
            }
        }
        if let Some(session) = &remote.activate_session {
            let destination = if let Some((tab, pane)) = &remote.activate_placement {
                remote
                    .layouts
                    .get(tab)
                    .filter(|layout| match pane {
                        Some(pane) => layout.root.leaves().iter().any(|leaf| {
                            &leaf.id == pane && leaf.session_id.as_ref() == Some(session)
                        }),
                        None => layout.drawer_shells().contains(session),
                    })
                    .map(|_| tab.clone())
            } else {
                remote
                    .layouts
                    .iter()
                    .find(|(_, layout)| layout.all_session_ids().contains(session))
                    .map(|(id, _)| id.clone())
            };
            if let Some(id) = destination {
                let layout = remote.layouts.get_mut(&id).unwrap();
                if let Some((_, pane)) = &remote.activate_placement {
                    if let Some(pane) = pane {
                        layout.focus_pane(pane);
                    } else {
                        layout.activate_drawer_shell(session);
                        layout.set_drawer_open(true);
                    }
                }
                remote.active = Some(Item::Tab(id));
                remote.activate_session = None;
                remote.activate_placement = None;
            }
        }
        if let Some(tab) = &remote.activate_tab {
            if remote.layouts.contains_key(tab) {
                remote.active = Some(Item::Tab(tab.clone()));
                remote.activate_tab = None;
            }
        }
        if let Some(mission) = &remote.activate_mission {
            if clients
                .read(cx)
                .overviews
                .get(&remote.computer.id)
                .is_some_and(|overview| overview.missions.iter().any(|item| &item.id == mission))
            {
                remote.active = Some(Item::Mission(mission.clone()));
                remote.activate_mission = None;
            }
        }
        if let Some(overview) = clients.read(cx).overviews.get(&remote.computer.id) {
            let valid = match &remote.active {
                Some(Item::Tab(id)) => remote.layouts.contains_key(id),
                Some(Item::Mission(id)) => {
                    overview.missions.iter().any(|mission| &mission.id == id)
                }
                Some(Item::Roles | Item::Crews) | None => true,
            };
            if !valid {
                remote.active = None;
            }
        }
        if remote.active.is_none() {
            if let Some(overview) = clients.read(cx).overviews.get(&remote.computer.id) {
                remote.active = overview
                    .tabs
                    .first()
                    .map(|tab| Item::Tab(tab.id.clone()))
                    .or_else(|| {
                        overview
                            .missions
                            .first()
                            .map(|mission| Item::Mission(mission.id.clone()))
                    });
            }
        }
        remote
            .connected
            .store(connection == Connection::Connected, Ordering::Release);
        if connection != Connection::Connected {
            remote.catalog = None;
            remote.library_catalog = None;
            remote.library_target = None;
            remote.role_surfaces.invalidate_library();
            remote.crew_surfaces.invalidate_library();
            remote.catalog_target = None;
            self.start_chat_modal = None;
            self.start_mission_modal = None;
            self.chat_rename_modal = None;
            self.pane_rename = None;
            self.fork_confirm = None;
            self.terminal_close_confirm = None;
            remote.project_picker = None;
            remote.project_picker_subscription = None;
            remote.delete_project = None;
            *remote.shell_placement.lock().unwrap() = None;
            remote.actions_pending.clear();
            remote.layout_hints.clear();
            remote.reopen_sessions.clear();
            remote.activate_session = None;
            remote.activate_placement = None;
            remote.activate_tab = None;
            remote.activate_mission = None;
            for pane in remote.terminals.values() {
                pane.input.update(cx, |input, cx| {
                    input.cancel_composition();
                    cx.notify();
                });
            }
        }
        if connection == Connection::Connected && remote.connection != connection {
            remote.errors.clear();
        }
        remote.connection = connection;
        let access = clients.update(cx, |state, cx| {
            state.front_page_access(&remote.computer, cx)
        });
        remote.front_page = crate::surfaces::front_page::FrontPageClient::remote(
            self.app_store.read(cx).client.clone(),
            access,
        );
        let sessions = clients
            .read(cx)
            .overviews
            .get(&remote.computer.id)
            .map(|overview| {
                overview
                    .sessions
                    .iter()
                    .map(|(id, session)| {
                        let native = remote.catalog.as_ref().is_some_and(|catalog| {
                            catalog.runtimes.iter().any(|runtime| {
                                Some(&runtime.name) == session.runtime.as_ref()
                                    && runtime.native_fork
                            })
                        });
                        let pinned = overview.tabs.iter().any(|tab| {
                            tab.pinned
                                && remote
                                    .layouts
                                    .get(&tab.id)
                                    .is_some_and(|layout| layout.contains_session(id))
                        });
                        (
                            id.clone(),
                            crate::surfaces::front_page::model::SessionDisplay::remote(
                                id,
                                session,
                                pinned,
                                native,
                                remote.front_page.supports(&["resume"]),
                            ),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        remote.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_front_page_target(remote.front_page.clone(), cx);
            sidebar.set_remote_presentation(
                remote.layouts.clone(),
                sessions,
                remote.active.clone(),
                cx,
            );
        });
        let workspace = remote.mission_workspace.clone();
        let mission = match &remote.active {
            Some(Item::Mission(id)) => Some(id.clone()),
            _ => None,
        };
        let overview = clients.read(cx).overviews.get(&remote.computer.id).cloned();
        let client = remote.front_page.clone();
        workspace.update(cx, |workspace, cx| {
            workspace.sync_remote(client, mission, overview, clients, cx)
        });
        self.ensure_remote_library(cx);
        self.load_remote_metadata(cx);
        self.refresh_remote_palette(cx);
        self.attach_remote_panes(cx);
        cx.notify();
    }
    fn load_remote_metadata(&mut self, cx: &mut Context<Self>) {
        let Some(remote) = self.remote_window.as_mut() else {
            return;
        };
        if !remote.front_page.supports(&["catalog"])
            || remote.catalog_target.as_ref() == Some(&remote.front_page.target)
        {
            return;
        }
        remote.catalog = None;
        remote.catalog_target = Some(remote.front_page.target.clone());
        let client = remote.front_page.clone();
        let guard = client.clone();
        let epoch = self.client_ui.epoch;
        let work = cx.background_spawn(async move { client.catalog() });
        cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |root, cx| {
                if root.client_ui.epoch != epoch || !guard.valid() {
                    return;
                }
                if let Some(remote) = root
                    .remote_window
                    .as_mut()
                    .filter(|remote| remote.front_page.target == guard.target)
                {
                    if let Ok(catalog) = result {
                        remote.catalog = Some(catalog);
                    }
                    root.sync_remote_state(cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }
    pub(crate) fn select_remote_item(&mut self, item: Item, cx: &mut Context<Self>) {
        if let Some(remote) = self.remote_window.as_mut() {
            if remote.active.as_ref() != Some(&item) {
                self.start_chat_modal = None;
                self.start_mission_modal = None;
                remote.form_generation = remote.form_generation.wrapping_add(1);
                remote.navigation_generation = remote.navigation_generation.wrapping_add(1);
                remote.activate_session = None;
                remote.activate_placement = None;
                remote.activate_tab = None;
                remote.activate_mission = None;
                for (id, hint) in &mut remote.layout_hints {
                    if let Some(previous) = remote.layouts.get(id) {
                        crate::surfaces::front_page::model::retain_presentation(hint, previous);
                    }
                }
                *remote.shell_placement.lock().unwrap() = None;
                remote.action_error = None;
                remote.recovery.clear();
            }
            remote.active = Some(item);
            remote.errors.clear();
        }
        self.sync_remote_state(cx);
        cx.notify();
    }
    pub(crate) fn attach_remote_panes(&mut self, cx: &mut Context<Self>) {
        let clients = state(&self.app_store, cx);
        let Some(remote) = self.remote_window.as_mut() else {
            return;
        };
        let overview = clients.read(cx).overviews.get(&remote.computer.id).cloned();
        let ids = overview
            .as_ref()
            .map(|overview| remote.visible_sessions(overview, cx))
            .unwrap_or_default();
        let running = overview
            .as_ref()
            .map(|overview| {
                overview
                    .sessions
                    .iter()
                    .filter(|(_, session)| {
                        !matches!(
                            session.status.status.effective(),
                            Status::Stopped | Status::Error
                        )
                    })
                    .map(|(id, _)| id)
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        remote.terminals.retain(|id, _| ids.contains(id));
        remote
            .pending
            .retain(|id, _| ids.contains(id) && running.contains(id));
        remote.errors.retain(|id| running.contains(id));
        if remote.connection != Connection::Connected {
            return;
        }
        for id in ids {
            if !running.contains(&id)
                || remote.terminals.contains_key(&id)
                || remote.pending.contains_key(&id)
                || remote.errors.contains(&id)
            {
                continue;
            }
            let Some(&(cols, rows)) = remote.sizes.get(&id) else {
                continue;
            };
            let request = Arc::new(());
            remote.pending.insert(id.clone(), request.clone());
            let config = RemoteTerminal {
                computer: remote.computer.id.clone(),
                session: id.clone(),
                cols,
                rows,
                take_control: !remote.recovery.contains(&id),
            };
            let address = config.address();
            let title = overview
                .as_ref()
                .and_then(|overview| overview.sessions.get(&id))
                .map(|session| session.title.clone())
                .unwrap_or_default();
            let (client, lifetime) = transport::RemoteTransport::client(
                self.app_store.read(cx).client.clone(),
                config,
                title,
                remote.connected.clone(),
            );
            let epoch = remote.epoch;
            let attachment_generation = remote.attachment_generation;
            let wake = remote.wake.clone();
            let wake_id = address.clone();
            let task = cx.background_spawn(async move {
                TerminalMirror::attach(
                    client,
                    address,
                    Arc::new(move || {
                        let _ = wake.unbounded_send(wake_id.clone());
                    }),
                )
            });
            cx.spawn(async move |weak, cx| {
                let result = task.await;
                let _ = weak.update(cx, |root, cx| {
                    let Some(remote) = root.remote_window.as_mut().filter(|remote| {
                        remote.epoch == epoch
                            && remote.attachment_generation == attachment_generation
                    }) else {
                        return;
                    };
                    if !remote
                        .pending
                        .get(&id)
                        .is_some_and(|pending| Arc::ptr_eq(pending, &request))
                    {
                        return;
                    }
                    remote.pending.remove(&id);
                    let clients = state(&root.app_store, cx);
                    let wanted = clients
                        .read(cx)
                        .overviews
                        .get(&remote.computer.id)
                        .is_some_and(|overview| {
                            remote.visible_sessions(overview, cx).contains(&id)
                                && overview.sessions.get(&id).is_some_and(|session| {
                                    !matches!(
                                        session.status.status.effective(),
                                        Status::Stopped | Status::Error
                                    )
                                })
                        });
                    if !wanted {
                        return;
                    }
                    match result {
                        Ok(terminal) => {
                            terminal.set_palette(app_settings::terminal_palette(
                                &root.app_store.read(cx).settings,
                                theme::active_variant(),
                            ));
                            let input = cx.new(|_| TerminalInput::new(terminal.clone()));
                            let input_address =
                                runner_core::remote::terminal_address(&remote.computer.id, &id);
                            let input_id = id.clone();
                            let input_terminal = terminal.clone();
                            let subscription = cx.observe(&input, move |root, input, cx| {
                                if let Some(Err(error)) =
                                    input.update(cx, |input, _| input.take_write_result())
                                {
                                    let current =
                                        root.remote_window.as_ref().is_some_and(|remote| {
                                            remote.epoch == epoch
                                                && remote.attachment_generation
                                                    == attachment_generation
                                                && remote.terminals.get(&input_id).is_some_and(
                                                    |pane| {
                                                        Arc::ptr_eq(&pane.terminal, &input_terminal)
                                                    },
                                                )
                                        });
                                    if current {
                                        root.report_remote_input_error(&input_address, error, cx);
                                    }
                                }
                                cx.notify();
                            });
                            let pane = RemotePane {
                                _lifetime: lifetime,
                                _view: terminal.view(),
                                interaction: cx.new(|_| TerminalInteraction::new(terminal.clone())),
                                scrollbar: cx.new(|_| Scrollbar::terminal(terminal.clone())),
                                input,
                                focus: cx.focus_handle(),
                                terminal,
                                _subscription: subscription,
                                control_failed: false,
                            };
                            remote.recovery.remove(&id);
                            remote.terminals.insert(id.clone(), pane);
                        }
                        Err(_) => {
                            remote.errors.insert(id.clone());
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }
    pub(crate) fn render_remote_connection(&self, _cx: &mut Context<Self>) -> AnyElement {
        let remote = self.remote_window.as_ref().unwrap();
        let (key, glyph, color) = match remote.connection {
            Connection::Connected => ("client.connection_live", "shield-check.svg", theme::muted()),
            Connection::Opening => (
                "client.connection_opening",
                "refresh-cw.svg",
                theme::muted(),
            ),
            Connection::Reconnecting => (
                "client.connection_reconnecting",
                "refresh-cw.svg",
                theme::warning(),
            ),
            Connection::Unreachable => (
                "client.connection_unreachable",
                "cloud-off.svg",
                theme::warning(),
            ),
            Connection::Revoked => ("client.connection_revoked", "lock.svg", theme::danger()),
            Connection::Forgotten => ("client.connection_forgotten", "unplug.svg", theme::muted()),
        };
        div()
            .debug_selector(|| "REMOTE_CONNECTION_ROW".into())
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap_2()
            .text_size(theme::text_meta())
            .text_color(color)
            .child(icon(glyph, color))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .child(label(key, &remote.computer.name)),
            )
            .into_any_element()
    }
    pub(crate) fn render_remote_workspace(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mission = self
            .remote_window
            .as_ref()
            .is_some_and(|remote| matches!(remote.active, Some(Item::Mission(_))));
        let library = self
            .remote_window
            .as_ref()
            .and_then(|remote| remote.active.clone());
        let body = if matches!(library, Some(Item::Roles)) {
            self.render_roles_page(window, cx)
        } else if matches!(library, Some(Item::Crews)) {
            self.render_crews_page(window, cx)
        } else if mission {
            let remote = self.remote_window.as_ref().unwrap();
            let mut sizes = remote.sizes.clone();
            sizes.extend(
                remote
                    .terminals
                    .iter()
                    .map(|(id, pane)| (id.clone(), pane.terminal.size())),
            );
            remote.mission_workspace.update(cx, |workspace, _| {
                workspace.set_remote_terminal_sizes(sizes)
            });
            remote.mission_workspace.clone().into_any_element()
        } else {
            self.render_active_tab(window, cx)
        };
        let remote = self.remote_window.as_ref().unwrap();
        let retry_shell = remote
            .shell_placement
            .lock()
            .unwrap()
            .as_ref()
            .map(|(_, layout)| layout.clone());
        let retry_root = cx.entity();
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(body)
            .children(remote.action_error.clone().map(|error| {
                runner_app::ui::notice_banner(error, runner_app::ui::Tone::Danger)
                    .debug_selector(|| "REMOTE_ACTION_ERROR".into())
            }))
            .children(
                retry_shell
                    .filter(|_| {
                        remote.action_error.is_some() && remote.front_page.supports(&["layout"])
                    })
                    .map(|draft| {
                        Button::new("retry-shell-placement", crate::i18n::text("ui.retry"))
                            .on_press(move |_, cx| {
                                retry_root.update(cx, |root, cx| {
                                    root.remote_start_shell(
                                        draft.clone(),
                                        None,
                                        (INITIAL_COLS, INITIAL_ROWS),
                                        cx,
                                    )
                                })
                            })
                    }),
            )
            .children((remote.connection == Connection::Reconnecting).then(|| {
                div()
                    .absolute()
                    .top(rems(74. / 16.))
                    .left_0()
                    .right_0()
                    .px_4()
                    .py_2()
                    .bg(theme::with_alpha(theme::warning(), 0.12))
                    .text_color(theme::warning())
                    .text_size(theme::text_body())
                    .child(label("client.reconnecting_banner", &remote.computer.name))
            }))
            .children(
                matches!(
                    remote.connection,
                    Connection::Unreachable | Connection::Revoked | Connection::Forgotten
                )
                .then(|| self.remote_connection_card(cx)),
            )
            .into_any_element()
    }
    pub(crate) fn render_remote_terminal_content(
        &self,
        id: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let remote = self.remote_window.as_ref().unwrap();
        let clients = state(&self.app_store, cx);
        let running = clients
            .read(cx)
            .overviews
            .get(&remote.computer.id)
            .and_then(|overview| overview.sessions.get(id))
            .is_some_and(|session| {
                !matches!(
                    session.status.status.effective(),
                    Status::Stopped | Status::Error
                )
            });
        let mut surface = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0();
        let Some(pane) = remote.terminals.get(id) else {
            let failed = remote.errors.contains(id);
            let owner = cx.weak_entity();
            let session = id.to_owned();
            let epoch = remote.epoch;
            let style = crate::surfaces::app_shell::terminal_style_for(self.settings(cx));
            let measure = gpui::canvas(
                move |bounds, window, cx| {
                    let font = window.text_system().resolve_font(&style.font);
                    let cell = window
                        .text_system()
                        .em_advance(font, px(style.font_size))
                        .unwrap_or(px(style.font_size * 0.6));
                    if let Some(size) = runner_app::terminal_resize::terminal_grid_size(
                        f32::from(bounds.size.width),
                        f32::from(bounds.size.height),
                        f32::from(cell),
                        (style.font_size * crate::terminal::element::LINE_HEIGHT_FACTOR).round(),
                    ) {
                        let owner = owner.clone();
                        let session = session.clone();
                        cx.defer(move |cx| {
                            let _ = owner.update(cx, |root, cx| {
                                if let Some(remote) = root
                                    .remote_window
                                    .as_mut()
                                    .filter(|remote| remote.epoch == epoch)
                                {
                                    let measured = (size.cols.max(20), size.rows.max(8));
                                    if remote.sizes.insert(session, measured) != Some(measured) {
                                        root.attach_remote_panes(cx);
                                        cx.notify();
                                    }
                                }
                            });
                        });
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0();
            if !running {
                return surface.child(measure).into_any_element();
            }
            return surface
                .child(
                    div()
                        .when(failed, |pane| {
                            pane.debug_selector(|| "REMOTE_TERMINAL_FAILED".into())
                        })
                        .relative()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_col()
                        .gap_3()
                        .children(
                            (!failed).then(|| {
                                spinner("remote-terminal-connecting", 24., theme::muted())
                            }),
                        )
                        .child(measure)
                        .child(label(
                            if failed {
                                "client.terminal_failed"
                            } else {
                                "client.terminal_connecting"
                            },
                            &remote.computer.name,
                        )),
                )
                .into_any_element();
        };
        let connected = remote.connection == Connection::Connected;
        let controlled = pane.terminal.control().desktop();
        let style = crate::surfaces::app_shell::terminal_style_for(self.settings(cx));
        if pane.terminal.palette() != style.palette {
            pane.terminal.set_palette(style.palette);
        }
        let terminal = pane.terminal.clone();
        let key_terminal = terminal.clone();
        let copy_terminal = terminal.clone();
        let paste_terminal = terminal.clone();
        let input = pane.input.clone();
        let take = cx.weak_entity();
        let take_id = id.to_owned();
        if !controlled {
            let owner_name =
                match &terminal.control().controller {
                    runner_core::protocol::terminal::TerminalController::Browser {
                        name, ..
                    } if name != "Host desktop" => name.clone(),
                    _ => remote.computer.name.clone(),
                };
            surface = surface.child(
                div()
                    .debug_selector(|| "REMOTE_TAKEN_BACK".into())
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .bg(theme::panel())
                    .text_size(theme::text_meta())
                    .child(div().flex_1().min_w_0().child(
                        if terminal.control().controller
                            == runner_core::protocol::terminal::TerminalController::None
                        {
                            crate::i18n::text("client.no_controller").to_string()
                        } else {
                            label("client.taken_back", &owner_name)
                        },
                    ))
                    .child(
                        Button::new(
                            SharedString::from(format!("take-remote-{id}")),
                            crate::i18n::text("client.take_control"),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Secondary)
                        .disabled(!connected)
                        .on_press(move |_, cx| {
                            let _ =
                                take.update(cx, |root, cx| root.take_remote_control(&take_id, cx));
                        }),
                    ),
            );
        }
        if pane.control_failed {
            surface = surface.child(
                div()
                    .debug_selector(|| "REMOTE_CONTROL_FAILED".into())
                    .px_3()
                    .py_2()
                    .text_size(theme::text_meta())
                    .text_color(theme::danger())
                    .child(crate::i18n::text("client.take_control_failed")),
            );
        }
        let interactive = connected && controlled && running;
        let option_as_meta = self.settings(cx).option_as_meta;
        surface
            .child(
                div().relative().flex_1().min_h_0().min_w_0().child(
                    crate::surfaces::panes::TerminalSurface {
                        terminal,
                        interaction: pane.interaction.clone(),
                        input: pane.input.clone(),
                        focus: pane.focus.clone(),
                        scrollbar: pane.scrollbar.clone(),
                        style,
                        interactive,
                        resize_owner: interactive,
                        scrollable: true,
                    }
                    .render()
                    .id(SharedString::from(format!("remote-terminal-{id}")))
                    .opacity(if interactive { 1. } else { 0.45 })
                    .on_key_down(move |event: &KeyDownEvent, _, cx| {
                        if !interactive {
                            cx.stop_propagation();
                            return;
                        }
                        let option_is_meta = option_as_meta.is_meta(cx);
                        let _ = crate::terminal::input::handle_key(
                            &key_terminal,
                            input.read(cx).is_composing(),
                            &event.keystroke,
                            option_is_meta,
                            cx,
                        );
                    })
                    .on_action(move |_: &Copy, _, cx| {
                        if !crate::terminal::input::copy_selection(&copy_terminal, cx) {
                            #[cfg(any(windows, target_os = "linux"))]
                            cx.propagate();
                        }
                    })
                    .on_action(move |_: &Paste, _, cx| {
                        if interactive {
                            let _ = crate::terminal::input::paste_text_from_clipboard(
                                &paste_terminal,
                                cx,
                            );
                        }
                        cx.stop_propagation();
                    }),
                ),
            )
            .into_any_element()
    }
    pub(super) fn take_remote_control(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(remote) = self.remote_window.as_mut() else {
            return;
        };
        let Some(pane) = remote.terminals.get_mut(id) else {
            return;
        };
        let terminal = pane.terminal.clone();
        let (cols, rows) = pane
            .interaction
            .read(cx)
            .pane_size
            .map(|size| (size.cols, size.rows))
            .unwrap_or_else(|| terminal.size());
        let epoch = remote.epoch;
        pane.control_failed = false;
        let taking = terminal.clone();
        let task = cx.background_spawn(async move { taking.take_back(cols, rows) });
        let id = id.to_owned();
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |root, cx| {
                let Some(remote) = root
                    .remote_window
                    .as_mut()
                    .filter(|remote| remote.epoch == epoch)
                else {
                    return;
                };
                let Some(pane) = remote
                    .terminals
                    .get_mut(&id)
                    .filter(|pane| Arc::ptr_eq(&pane.terminal, &terminal))
                else {
                    return;
                };
                pane.control_failed = result.is_err();
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn remote_connection_card(&self, cx: &mut Context<Self>) -> AnyElement {
        let remote = self.remote_window.as_ref().unwrap();
        let revoked = remote.connection == Connection::Revoked;
        let forgotten = remote.connection == Connection::Forgotten;
        let pair = revoked || forgotten;
        let retry = cx.weak_entity();
        let close = cx.weak_entity();
        let computer = remote.computer.clone();
        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::with_alpha(theme::bg(), 0.7))
            .p_6()
            .child(
                div()
                    .debug_selector(|| "REMOTE_CONNECTION_CARD".into())
                    .w_full()
                    .max_w(rems(440. / 16.))
                    .rounded_xl()
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .p_6()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_4()
                    .text_center()
                    .child(icon(
                        if revoked {
                            "shield-x.svg"
                        } else {
                            "cloud-off.svg"
                        },
                        if revoked {
                            theme::danger()
                        } else {
                            theme::muted()
                        },
                    ))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(theme::text_lead())
                            .child(label(
                                if forgotten {
                                    "client.forgotten_title"
                                } else if revoked {
                                    "client.revoked_title"
                                } else {
                                    "client.unreachable_title"
                                },
                                &computer.name,
                            )),
                    )
                    .child(
                        div()
                            .text_size(theme::text_body())
                            .text_color(theme::muted())
                            .child(label(
                                if forgotten {
                                    "client.forgotten_detail"
                                } else if revoked {
                                    "client.revoked_detail"
                                } else {
                                    "client.unreachable_detail"
                                },
                                &computer.name,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                Button::new(
                                    "remote-reconnect",
                                    crate::i18n::text(if forgotten {
                                        "client.pair"
                                    } else if revoked {
                                        "client.pair_again"
                                    } else {
                                        "client.reconnect"
                                    }),
                                )
                                .on_press(move |window, cx| {
                                    let _ = retry.update(cx, |root, cx| {
                                        if pair {
                                            root.open_computer_pair(
                                                computer.clone(),
                                                pairing::Purpose::Open { new_window: false },
                                                window,
                                                cx,
                                            );
                                        } else {
                                            let clients = state(&root.app_store, cx);
                                            clients.update(cx, |clients, cx| {
                                                clients.connections.insert(
                                                    computer.id.clone(),
                                                    Connection::Opening,
                                                );
                                                clients.send(Command::Retry(computer.id.clone()));
                                                cx.notify();
                                            });
                                        }
                                    });
                                }),
                            )
                            .child(
                                Button::new("remote-close-view", crate::i18n::text("ui.close_tab"))
                                    .variant(ButtonVariant::Ghost)
                                    .on_press(move |window, cx| {
                                        let _ = close.update(cx, |root, cx| {
                                            root.return_to_this_computer(window, cx)
                                        });
                                    }),
                            ),
                    ),
            )
            .into_any_element()
    }
}
pub(crate) fn sidebar_nodes(overview: &Overview) -> Vec<SidebarNode> {
    let nodes = overview.sidebar.clone().unwrap_or_else(|| {
        let mut nodes = overview
            .projects
            .iter()
            .enumerate()
            .map(|(position, project)| SidebarNode {
                kind: SidebarNodeKind::Project,
                id: project.id.clone(),
                parent: None,
                position: position as i64,
                pinned_position: None,
            })
            .collect::<Vec<_>>();
        for (position, tab) in overview.tabs.iter().enumerate() {
            nodes.push(SidebarNode {
                kind: SidebarNodeKind::Tab,
                id: tab.id.clone(),
                parent: tab.project.clone(),
                position: position as i64,
                pinned_position: tab.pinned.then_some(position as i64),
            });
        }
        for (position, mission) in overview.missions.iter().enumerate() {
            let position = (overview.tabs.len() + position) as i64;
            nodes.push(SidebarNode {
                kind: SidebarNodeKind::Mission,
                id: mission.id.clone(),
                parent: mission.project.clone(),
                position,
                pinned_position: mission.pinned.then_some(position),
            });
        }
        nodes
    });
    nodes
}
