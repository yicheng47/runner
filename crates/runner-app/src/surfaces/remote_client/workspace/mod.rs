mod layout;
mod render;
mod sync;

pub(crate) use layout::*;

use super::*;

use runner_app::ui::button::spinner;

use runner_core::companion::{Overview, SidebarNode, SidebarNodeKind, Status};

use runner_core::remote::RemoteTerminal;

use std::sync::atomic::{AtomicBool, Ordering};

use std::sync::Mutex;

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum Item {
    Launcher,
    Roles,
    Crews,
    Tab(String),
    Mission(String),
}

pub(super) struct RemotePane {
    _lifetime: transport::AttachmentLifetime,
    pub(super) attachment: AttachedChat,
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
    _usage: super::usage::UsageWatch,
    pub(crate) active: Option<Item>,
    pub(crate) layouts: HashMap<String, PaneLayout>,
    pub(crate) overview_revision: u64,
    pub(crate) resize_draft: Option<PaneLayout>,
    pub(crate) resizing_split: Option<String>,
    pub(crate) drawer_resizing: bool,
    pub(crate) pane_bounds: HashMap<PaneKey, gpui::Size<Pixels>>,
    pub(crate) layout_hints: HashMap<String, PaneLayout>,
    // Archive can push an empty pane before the collapsed tree arrives.
    pub(crate) closing_layouts: HashMap<String, PaneLayout>,
    pub(crate) actions_pending: HashSet<String>,
    pub(crate) action_error: Option<String>,
    pub(crate) shell_placement: super::actions::ShellPlacement,
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
    /// Tabs in the order they were shown, the most recent last.
    pub(crate) viewed_tabs: Vec<String>,
    pub(crate) landed_at: Option<std::time::Instant>,
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
            None | Some(Item::Launcher | Item::Roles | Item::Crews) => Vec::new(),
        }
    }

    /// Running chats in open tabs keep their attachment while hidden and
    /// connected, so switching back needs no attach, take or host resize, and
    /// keep their recovery marker until they reattach. A hidden pane is not
    /// drawn, so it sends no viewport or size.
    fn open_sessions(&self, overview: &Overview) -> HashSet<String> {
        self.layouts
            .values()
            .flat_map(PaneLayout::all_session_ids)
            .filter(|id| {
                overview.sessions.get(id).is_some_and(|session| {
                    !matches!(
                        session.status.status.effective(),
                        Status::Stopped | Status::Error
                    )
                })
            })
            .collect()
    }
}

impl RemoteClient {
    /// Changes whenever the host connection is reset or lost: a re-pair, a
    /// host daemon restart, a dropped or restored connection.
    pub(crate) fn connection_marker(&self) -> Option<(u64, bool)> {
        self.workspace.as_ref().map(|remote| {
            (
                remote.attachment_generation,
                remote.connection == Connection::Connected,
            )
        })
    }

    pub(crate) fn report_remote_input_error(
        &mut self,
        address: &str,
        message: String,
        cx: &mut Context<NativeRoot>,
    ) {
        let Some((computer, session)) = runner_core::remote::terminal_target(address) else {
            return;
        };
        let clients = state(&self.app_store, cx);
        let Some(remote) = self.workspace.as_mut().filter(|remote| {
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

    pub(crate) fn shows_remote_terminal(&self, address: &str, cx: &App) -> bool {
        let Some((computer, session)) = runner_core::remote::terminal_target(address) else {
            return false;
        };
        self.workspace.as_ref().is_some_and(|remote| {
            remote.computer.id == computer
                && state(&self.app_store, cx)
                    .read(cx)
                    .overviews
                    .get(computer)
                    .is_some_and(|overview| {
                        remote
                            .visible_sessions(overview, cx)
                            .iter()
                            .any(|id| id == session)
                    })
        })
    }

    pub(crate) fn remote_terminal_interactive(&self, id: &str, cx: &App) -> bool {
        self.workspace.as_ref().is_some_and(|remote| {
            remote.connection == Connection::Connected
                && remote
                    .terminals
                    .get(id)
                    .is_some_and(|pane| pane.attachment.terminal.control().desktop())
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
        let remote = self.workspace.as_ref()?;
        remote
            .terminals
            .get(id)
            .map(|pane| pane.attachment.terminal.size())
            .or_else(|| remote.sizes.get(id).copied())
    }

    pub(crate) fn remote_terminal_palette(
        &self,
        cx: &App,
    ) -> Option<runner_terminal::palette::TerminalPalette> {
        let remote = self.workspace.as_ref()?;
        state(&self.app_store, cx)
            .read(cx)
            .overviews
            .get(&remote.computer.id)?
            .terminal_palette
            .clone()
            .map(|palette| runner_terminal::terminal::palette_from_wire(palette.into()))
    }

    pub(crate) fn configure_remote_terminals(&self, cx: &App) {
        let Some(remote) = self.workspace.as_ref() else {
            return;
        };
        for pane in remote.terminals.values() {
            app_settings::configure_terminal(&pane.attachment.terminal, self.settings(cx));
        }
    }

    pub(crate) fn focus_remote_terminal(&self, id: &str, window: &mut Window, cx: &mut App) {
        if let Some(pane) = self
            .workspace
            .as_ref()
            .and_then(|remote| remote.terminals.get(id))
        {
            pane.attachment.terminal_focus.focus(window, cx);
        }
    }

    pub(crate) fn visit_computer(
        &mut self,
        computer: Computer,
        window_label: &str,
        window: &mut Window,
        cx: &mut Context<NativeRoot>,
    ) -> bool {
        if let Some(remote) = self
            .workspace
            .as_mut()
            .filter(|remote| remote.computer.id == computer.id)
        {
            remote.computer = computer.clone();
            self.ui.picker_open = false;
            if !matches!(
                remote.connection,
                Connection::Unreachable | Connection::Revoked | Connection::Forgotten
            ) {
                cx.notify();
                return false;
            }
            remote.terminals.clear();
            remote.pending.clear();
            remote.recovery.clear();
            remote.attachment_generation = remote.attachment_generation.wrapping_add(1);
            remote.errors.clear();
            state(&self.app_store, cx)
                .read(cx)
                .send(Command::Retry(computer.id));
            return false;
        }
        self.remember_remote_selection();
        if let Some(remote) = &self.workspace {
            remote
                .mission_workspace
                .update(cx, |workspace, cx| workspace.release_window(cx));
        }
        self.ui.epoch += 1;
        let epoch = self.ui.epoch;
        let clients = state(&self.app_store, cx);
        let follow = clients.read(cx).follow(computer.id.clone());
        let usage = clients.read(cx).watch_usage(computer.id.clone(), false);
        let (wake, mut wakes) = futures::channel::mpsc::unbounded();
        cx.spawn(async move |weak, cx| {
            while let Some(id) = wakes.next().await {
                if weak
                    .update(cx, |root, cx| {
                        if root
                            .remote_client
                            .workspace
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
        let selection = self
            .ui
            .selections
            .remove(&computer.id)
            .filter(|_| clients.read(cx).overviews.contains_key(&computer.id))
            .unwrap_or_default();
        let shell = cx.weak_entity();
        let sidebar =
            cx.new(|sidebar_cx| Sidebar::new(shell, self.app_store.clone(), None, sidebar_cx));
        sidebar.update(cx, |sidebar, cx| {
            sidebar.set_front_page_target(front_page.clone(), cx)
        });
        self.restore_pending_archive_marks(&computer.id, &sidebar, cx);
        let shell = cx.weak_entity();
        let mission_workspace = cx.new(|workspace_cx| {
            MissionWorkspace::new_remote(
                window_label.to_owned(),
                shell,
                self.app_store.clone(),
                front_page.clone(),
                window,
                workspace_cx,
            )
        });
        self.workspace = Some(RemoteWindow {
            mission_workspace,
            sidebar,
            sidebar_collapsed: false,
            sidebar_width: self.settings(cx).sidebar_width,
            front_page,
            computer,
            _follow: Some(follow),
            _usage: usage,
            active: selection.active,
            layouts: selection.layouts,
            overview_revision: 0,
            resize_draft: None,
            resizing_split: None,
            drawer_resizing: false,
            pane_bounds: HashMap::new(),
            layout_hints: HashMap::new(),
            actions_pending: HashSet::new(),
            closing_layouts: HashMap::new(),
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
            viewed_tabs: Vec::new(),
            landed_at: None,
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
        self.ui.picker_open = false;
        self.root_focus.focus(window, cx);
        cx.notify();
        true
    }

    pub(crate) fn return_to_this_computer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<NativeRoot>,
    ) {
        self.remember_remote_selection();
        if let Some(remote) = &self.workspace {
            remote
                .mission_workspace
                .update(cx, |workspace, cx| workspace.release_window(cx));
        }
        self.workspace = None;
        self.ui.picker_open = false;
        self.root_focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn remember_remote_selection(&mut self) {
        if let Some(remote) = &self.workspace {
            self.ui.selections.insert(
                remote.computer.id.clone(),
                RemoteSelection {
                    active: remote.active.clone(),
                    layouts: remote.layouts.clone(),
                },
            );
        }
    }

    pub(crate) fn remote_mission_workspace(&self) -> Option<Entity<MissionWorkspace>> {
        let remote = self.workspace.as_ref()?;
        matches!(
            remote.active,
            Some(crate::surfaces::remote_client::RemoteItem::Mission(_))
        )
        .then(|| remote.mission_workspace.clone())
    }
}
