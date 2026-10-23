use super::*;
use runner_app::ui::button::spinner;
use runner_core::companion::{Overview, Pane, SplitDirection, Status};
use runner_core::remote::RemoteTerminal;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, PartialEq, Eq)]
pub(super) enum Item {
    Tab(String),
    Mission(String),
    Shell(String),
}
pub(super) struct RemotePane {
    _lifetime: transport::AttachmentLifetime,
    pub(super) terminal: Arc<TerminalMirror>,
    _view: TerminalView,
    interaction: Entity<TerminalInteraction>,
    input: Entity<TerminalInput>,
    focus: FocusHandle,
    scrollbar: Entity<Scrollbar>,
    _subscription: Subscription,
    control_failed: bool,
}
pub(crate) struct RemoteWindow {
    pub(crate) computer: Computer,
    _follow: model::Follow,
    active: Option<Item>,
    pub(super) terminals: HashMap<String, RemotePane>,
    recovery: HashSet<String>,
    terminal_epoch: u64,
    pair_epoch: u64,
    attachment_generation: u64,
    pending: HashSet<String>,
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
    fn tree(&self, overview: &Overview) -> Option<Pane> {
        match self.active.as_ref()? {
            Item::Tab(id) => overview
                .tabs
                .iter()
                .find(|tab| &tab.id == id)
                .map(|tab| tab.panes.clone()),
            Item::Mission(id) => overview
                .missions
                .iter()
                .find(|mission| &mission.id == id)
                .map(|mission| mission_panes(&mission.members)),
            Item::Shell(id) => Some(Pane::Leaf {
                session: Some(id.clone()),
            }),
        }
    }
}
pub(super) fn mission_panes(members: &[runner_core::companion::Member]) -> Pane {
    if members.len() <= 1 {
        return Pane::Leaf {
            session: members.first().map(|member| member.session.clone()),
        };
    }
    let mid = members.len() / 2;
    Pane::Split {
        split: SplitDirection::Row,
        sizes: [mid as f64, (members.len() - mid) as f64],
        a: Box::new(mission_panes(&members[..mid])),
        b: Box::new(mission_panes(&members[mid..])),
    }
}

fn session_ids(pane: &Pane, ids: &mut Vec<String>) {
    match pane {
        Pane::Split { a, b, .. } => {
            session_ids(a, ids);
            session_ids(b, ids);
        }
        Pane::Leaf { session: Some(id) } => ids.push(id.clone()),
        _ => {}
    }
}

impl NativeRoot {
    pub(crate) fn visit_computer(
        &mut self,
        computer: Computer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        self.remote_window = Some(RemoteWindow {
            computer,
            _follow: follow,
            active: None,
            terminals: HashMap::new(),
            recovery: HashSet::new(),
            terminal_epoch: clients.read(cx).terminal_epoch,
            pair_epoch,
            attachment_generation: 0,
            pending: HashSet::new(),
            sizes: HashMap::new(),
            errors: HashSet::new(),
            connected: Arc::new(AtomicBool::new(false)),
            connection: Connection::Opening,
            epoch,
            wake,
        });
        self.client_ui.picker_open = false;
        self.attached.clear();
        self.mission_workspace
            .update(cx, |workspace, cx| workspace.release_window(cx));
        self.root_focus.focus(window, cx);
        self.report_current_subjects(cx);
        self.sync_remote_state(cx);
        cx.notify();
    }
    pub(crate) fn return_to_this_computer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            remote.recovery.extend(remote.pending.iter().cloned());
            remote.terminals.clear();
            remote.pending.clear();
            remote.errors.clear();
            remote.terminal_epoch = terminal_epoch;
            remote.attachment_generation = remote.attachment_generation.wrapping_add(1);
        }
        let connection = clients.read(cx).connection(&remote.computer.id);
        if connection == Connection::Forgotten && remote.connection != connection {
            remote.terminals.clear();
            remote.pending.clear();
            remote.recovery.clear();
            remote.errors.clear();
            remote.attachment_generation = remote.attachment_generation.wrapping_add(1);
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
        self.attach_remote_panes(cx);
        cx.notify();
    }
    pub(super) fn select_remote_item(&mut self, item: Item, cx: &mut Context<Self>) {
        if let Some(remote) = self.remote_window.as_mut() {
            if remote.active.as_ref() != Some(&item) {
                remote.recovery.clear();
            }
            remote.active = Some(item);
            remote.errors.clear();
        }
        self.attach_remote_panes(cx);
        cx.notify();
    }
    fn attach_remote_panes(&mut self, cx: &mut Context<Self>) {
        let clients = state(&self.app_store, cx);
        let Some(remote) = self.remote_window.as_mut() else {
            return;
        };
        let overview = clients.read(cx).overviews.get(&remote.computer.id).cloned();
        let mut ids = Vec::new();
        if let Some(tree) = overview.as_ref().and_then(|overview| remote.tree(overview)) {
            session_ids(&tree, &mut ids);
        }
        remote.terminals.retain(|id, _| ids.contains(id));
        if remote.connection != Connection::Connected {
            return;
        }
        for id in ids {
            if remote.terminals.contains_key(&id)
                || remote.pending.contains(&id)
                || remote.errors.contains(&id)
            {
                continue;
            }
            let Some(&(cols, rows)) = remote.sizes.get(&id) else {
                continue;
            };
            remote.pending.insert(id.clone());
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
                    remote.pending.remove(&id);
                    match result {
                        Ok(terminal) => {
                            let clients = state(&root.app_store, cx);
                            let mut desired = Vec::new();
                            if let Some(tree) = clients
                                .read(cx)
                                .overviews
                                .get(&remote.computer.id)
                                .and_then(|overview| remote.tree(overview))
                            {
                                session_ids(&tree, &mut desired);
                            }
                            if !desired.contains(&id) {
                                return;
                            }
                            terminal.set_palette(app_settings::terminal_palette(
                                &root.app_store.read(cx).settings,
                                theme::active_variant(),
                            ));
                            let input = cx.new(|_| TerminalInput::new(terminal.clone()));
                            let subscription = cx.observe(&input, |_, input, cx| {
                                input.update(cx, |input, _| {
                                    let _ = input.take_write_result();
                                });
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
    pub(crate) fn render_remote_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let remote = self.remote_window.as_ref().unwrap();
        let clients = state(&self.app_store, cx);
        let overview = clients.read(cx).overviews.get(&remote.computer.id).cloned();
        let mut content = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .px_5()
                    .pb_3()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .opacity(0.35)
                    .children(
                        [
                            ("message-square-plus.svg", "ui.new_chat"),
                            ("user.svg", "noun.role"),
                            ("users.svg", "noun.crew"),
                        ]
                        .map(|(glyph, key)| {
                            div()
                                .debug_selector(|| "REMOTE_DISABLED_CREATE".into())
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(icon(glyph, theme::muted()))
                                .child(crate::i18n::text(key))
                        }),
                    ),
            )
            .child(div().mx_4().h(px(1.)).bg(theme::border()));
        let mut rows = div()
            .id("remote-sidebar-scroll")
            .overflow_y_scroll()
            .flex_1()
            .min_h_0()
            .px_3()
            .py_3()
            .flex()
            .flex_col()
            .gap_1();
        if let Some(overview) = overview.as_ref() {
            rows = rows.child(section(crate::i18n::text("heading.projects")));
            for project in &overview.projects {
                rows = rows.child(
                    div()
                        .px_2()
                        .py_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_color(theme::muted())
                        .child(icon("folder.svg", theme::muted()))
                        .child(project.name.clone()),
                );
                for tab in overview
                    .tabs
                    .iter()
                    .filter(|tab| tab.project.as_deref() == Some(&project.id))
                {
                    rows = rows.child(self.remote_sidebar_row(
                        &tab.name,
                        Item::Tab(tab.id.clone()),
                        tab_attention(&tab.panes, overview),
                        cx,
                    ));
                }
                for mission in overview
                    .missions
                    .iter()
                    .filter(|mission| mission.project.as_deref() == Some(&project.id))
                {
                    rows = rows.child(self.remote_sidebar_row(
                        &mission.title,
                        Item::Mission(mission.id.clone()),
                        mission.asks.len(),
                        cx,
                    ));
                }
            }
            rows = rows.child(section(crate::i18n::text("client.recent")));
            for tab in overview.tabs.iter().filter(|tab| tab.project.is_none()) {
                rows = rows.child(self.remote_sidebar_row(
                    &tab.name,
                    Item::Tab(tab.id.clone()),
                    tab_attention(&tab.panes, overview),
                    cx,
                ));
            }
            rows = rows.child(section(crate::i18n::text("client.missions")));
            for mission in overview
                .missions
                .iter()
                .filter(|mission| mission.project.is_none())
            {
                rows = rows.child(self.remote_sidebar_row(
                    &mission.title,
                    Item::Mission(mission.id.clone()),
                    mission.asks.len(),
                    cx,
                ));
            }
        } else {
            for width in [160., 200., 130., 170.] {
                rows = rows.child(
                    div()
                        .mx_2()
                        .my_2()
                        .h(rems(12. / 16.))
                        .w(rems(width / 16.))
                        .max_w_full()
                        .rounded_sm()
                        .bg(theme::raised()),
                );
            }
        }
        content = content.child(rows);
        content.into_any_element()
    }
    fn remote_sidebar_row(
        &self,
        name: &str,
        item: Item,
        attention: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self
            .remote_window
            .as_ref()
            .is_some_and(|remote| remote.active.as_ref() == Some(&item));
        let key = match &item {
            Item::Tab(id) => format!("remote-tab-{id}"),
            Item::Mission(id) => format!("remote-mission-{id}"),
            Item::Shell(id) => format!("remote-shell-{id}"),
        };
        let (working, unread, needs_input, stopped) = {
            let clients = state(&self.app_store, cx);
            let overview = self
                .remote_window
                .as_ref()
                .and_then(|remote| clients.read(cx).overviews.get(&remote.computer.id));
            let mut ids = Vec::new();
            if let Some(overview) = overview {
                match &item {
                    Item::Tab(id) => {
                        if let Some(tab) = overview.tabs.iter().find(|tab| &tab.id == id) {
                            session_ids(&tab.panes, &mut ids);
                        }
                    }
                    Item::Mission(id) => {
                        if let Some(mission) =
                            overview.missions.iter().find(|mission| &mission.id == id)
                        {
                            ids.extend(mission.members.iter().map(|member| member.session.clone()));
                        }
                    }
                    Item::Shell(id) => ids.push(id.clone()),
                }
                let statuses = ids
                    .iter()
                    .filter_map(|id| overview.sessions.get(id))
                    .map(|session| &session.status)
                    .collect::<Vec<_>>();
                (
                    statuses
                        .iter()
                        .any(|s| matches!(s.status, Status::Working | Status::Starting)),
                    statuses.iter().any(|s| s.unread),
                    attention > 0 || statuses.iter().any(|s| s.status == Status::NeedsYou),
                    !statuses.is_empty()
                        && statuses
                            .iter()
                            .all(|s| matches!(s.status, Status::Stopped | Status::Error)),
                )
            } else {
                (false, false, attention > 0, false)
            }
        };
        div()
            .id(SharedString::from(key))
            .debug_selector(|| "REMOTE_SIDEBAR_ROW".into())
            .tab_index(0)
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .rounded(rems(6. / 16.))
            .bg(if selected {
                theme::chrome_selected()
            } else {
                gpui::transparent_black()
            })
            .cursor_pointer()
            .hover(|row| row.bg(theme::chrome_hover()))
            .on_click(cx.listener({
                let item = item.clone();
                move |root, _, _, cx| root.select_remote_item(item.clone(), cx)
            }))
            .on_key_down(cx.listener(move |root, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    root.select_remote_item(item.clone(), cx);
                    cx.stop_propagation();
                }
            }))
            .child(icon("message-square.svg", theme::muted()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(if unread {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(if stopped {
                        theme::faint()
                    } else {
                        theme::text()
                    })
                    .child(name.to_string()),
            )
            .children(needs_input.then(|| icon("hand.svg", theme::warning())))
            .children(
                (working && !needs_input)
                    .then(|| spinner("remote-sidebar-working", 12., theme::muted())),
            )
            .into_any_element()
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
        let remote = self.remote_window.as_ref().unwrap();
        let clients = state(&self.app_store, cx);
        let overview = clients.read(cx).overviews.get(&remote.computer.id).cloned();
        let tree = overview.as_ref().and_then(|overview| remote.tree(overview));
        let title = overview
            .as_ref()
            .and_then(|overview| match remote.active.as_ref()? {
                Item::Tab(id) => overview
                    .tabs
                    .iter()
                    .find(|tab| &tab.id == id)
                    .map(|tab| tab.name.clone()),
                Item::Mission(id) => overview
                    .missions
                    .iter()
                    .find(|mission| &mission.id == id)
                    .map(|mission| mission.title.clone()),
                Item::Shell(id) => overview
                    .sessions
                    .get(id)
                    .map(|session| session.title.clone()),
            })
            .unwrap_or_else(|| remote.computer.name.clone());
        let mut body = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .child(
                div()
                    .h(rems(44. / 16.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .px_4()
                    .border_b_1()
                    .border_color(theme::border())
                    .child(title),
            );
        if let Some(tree) = tree.as_ref() {
            let pane = self.render_remote_tree(tree, overview.as_ref().unwrap(), window, cx);
            body = body.child(div().relative().flex_1().min_h_0().flex().child(pane));
        } else {
            body = body.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .flex_col()
                    .gap_3()
                    .children(
                        (remote.connection == Connection::Opening)
                            .then(|| spinner("remote-opening", 24., theme::muted())),
                    )
                    .child(label(
                        if remote.connection == Connection::Opening {
                            "client.opening_title"
                        } else {
                            "client.choose_tab"
                        },
                        &remote.computer.name,
                    ))
                    .child(
                        div()
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .child(crate::i18n::text("client.opening_detail")),
                    ),
            );
        }
        if let Some(Item::Tab(id)) = &remote.active {
            if let Some(tab) = overview
                .as_ref()
                .and_then(|overview| overview.tabs.iter().find(|tab| &tab.id == id))
            {
                if !tab.shells.is_empty() {
                    body =
                        body.child(self.remote_shells(&tab.shells, overview.as_ref().unwrap(), cx));
                }
            }
        }
        if let Some(Item::Mission(id)) = &remote.active {
            if let Some(mission) = overview
                .as_ref()
                .and_then(|overview| overview.missions.iter().find(|mission| &mission.id == id))
            {
                if !mission.shells.is_empty() {
                    body = body.child(self.remote_shells(
                        &mission.shells,
                        overview.as_ref().unwrap(),
                        cx,
                    ));
                }
            }
        }
        if remote.connection == Connection::Reconnecting {
            body = body.child(
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
                    .child(label("client.reconnecting_banner", &remote.computer.name)),
            );
        }
        if matches!(
            remote.connection,
            Connection::Unreachable | Connection::Revoked | Connection::Forgotten
        ) {
            body = body.child(self.remote_connection_card(cx));
        }
        body.into_any_element()
    }
    fn remote_shells(
        &self,
        shells: &[String],
        overview: &Overview,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .debug_selector(|| "REMOTE_SHELLS".into())
            .flex()
            .gap_2()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(theme::border())
            .children(shells.iter().map(|id| {
                let item = Item::Shell(id.clone());
                let name = overview
                    .sessions
                    .get(id)
                    .map(|session| session.title.clone())
                    .unwrap_or_else(|| crate::i18n::text("noun.terminal").into());
                self.remote_sidebar_row(&name, item, 0, cx)
            }))
            .into_any_element()
    }
    fn render_remote_tree(
        &self,
        tree: &Pane,
        overview: &Overview,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match tree {
            Pane::Split { split, sizes, a, b } => {
                let column = *split == SplitDirection::Column;
                let total = sizes[0] + sizes[1];
                let ratio = if total.is_finite() && total > 0. {
                    (sizes[0] / total).clamp(0.05, 0.95) as f32
                } else {
                    0.5
                };
                div()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .when(column, |pane| pane.flex_col())
                    .child(
                        div()
                            .relative()
                            .min_w_0()
                            .min_h_0()
                            .flex_none()
                            .when(column, |pane| pane.h(relative(ratio)).w_full())
                            .when(!column, |pane| pane.w(relative(ratio)).h_full())
                            .child(self.render_remote_tree(a, overview, window, cx)),
                    )
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .when(column, |pane| pane.border_t_1())
                            .when(!column, |pane| pane.border_l_1())
                            .border_color(theme::border())
                            .child(self.render_remote_tree(b, overview, window, cx)),
                    )
                    .into_any_element()
            }
            Pane::Leaf { session: Some(id) } => {
                self.render_remote_terminal(id, overview, window, cx)
            }
            _ => div().size_full().into_any_element(),
        }
    }
    fn render_remote_terminal(
        &self,
        id: &str,
        overview: &Overview,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let remote = self.remote_window.as_ref().unwrap();
        let name = overview
            .sessions
            .get(id)
            .map(|session| session.title.clone())
            .unwrap_or_default();
        let header_id = id.to_owned();
        let mut surface = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .child(
                div()
                    .debug_selector(move || format!("REMOTE_TERMINAL_HEADER_{header_id}"))
                    .h(rems(30. / 16.))
                    .px_3()
                    .flex()
                    .items_center()
                    .flex_none()
                    .bg(theme::panel())
                    .border_b_1()
                    .border_color(theme::border())
                    .text_size(theme::text_body())
                    .child(name),
            );
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
        let running = overview.sessions.get(id).is_some_and(|session| {
            !matches!(session.status.status, Status::Stopped | Status::Error)
        });
        let interactive = connected && controlled && running;
        let option_as_meta = self.settings(cx).option_as_meta;
        surface
            .child(
                div()
                    .id(SharedString::from(format!("remote-terminal-{id}")))
                    .relative()
                    .key_context("Terminal")
                    .track_focus(&pane.focus)
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .py_3()
                    .pl_3()
                    .pr_1()
                    .opacity(if interactive { 1. } else { 0.45 })
                    .on_key_down(move |event: &KeyDownEvent, _, cx| {
                        if !interactive {
                            cx.stop_propagation();
                            return;
                        }
                        let key = &event.keystroke;
                        let option_is_meta = option_as_meta.is_meta(cx);
                        if runner_app::terminal_ime::swallows_option_copy(key, option_is_meta) {
                            cx.stop_propagation();
                            return;
                        }
                        if runner_app::terminal_ime::terminal_key_route(
                            input.read(cx).is_composing(),
                            key,
                            option_is_meta,
                        ) == runner_app::terminal_ime::TerminalKeyRoute::Raw
                            && key_terminal
                                .send_key(
                                    &key.key,
                                    key.modifiers.control,
                                    key.modifiers.alt,
                                    key.modifiers.shift,
                                    key.key_char.as_deref(),
                                )
                                .unwrap_or(false)
                        {
                            key_terminal.scroll_to_bottom();
                            cx.stop_propagation();
                        }
                    })
                    .on_action(move |_: &Copy, _, cx| {
                        let Some(text) = copy_terminal.selection_text() else {
                            #[cfg(windows)]
                            cx.propagate();
                            return;
                        };
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                        cx.stop_propagation();
                    })
                    .on_action(move |_: &Paste, _, cx| {
                        if interactive {
                            if let Some(text) =
                                cx.read_from_clipboard().and_then(|item| item.text())
                            {
                                let _ = paste_terminal.paste(&text);
                            }
                        }
                        cx.stop_propagation();
                    })
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .pr(runner_app::ui::terminal_scrollbar_gutter())
                            .child(
                                TerminalElement::new(
                                    terminal,
                                    pane.interaction.clone(),
                                    pane.input.clone(),
                                    pane.focus.clone(),
                                    interactive,
                                    interactive,
                                    style,
                                )
                                .scrollable(true)
                                .cached(),
                            )
                            .child(pane.scrollbar.clone()),
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
fn section(title: &str) -> gpui::Div {
    div()
        .px_2()
        .pt_4()
        .pb_2()
        .text_size(theme::text_meta())
        .text_color(theme::faint())
        .child(title.to_string())
}
fn tab_attention(pane: &Pane, overview: &Overview) -> usize {
    let mut ids = Vec::new();
    session_ids(pane, &mut ids);
    ids.iter()
        .filter(|id| {
            overview
                .sessions
                .get(*id)
                .is_some_and(|session| session.status.status == Status::NeedsYou)
        })
        .count()
}
