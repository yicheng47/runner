mod actions;
mod bridge;
pub(crate) mod feed;
mod library;
pub(crate) mod model;
mod pairing;
mod picker;
mod project;
mod start_chat;
#[cfg(test)]
pub(crate) mod tests;
pub(crate) mod transport;
mod usage;
mod workspace;

use crate::*;
use gpui::{svg, FontWeight};
use model::{computer_rows, state, Access, Command, Connection};
use runner_app::ui::ButtonVariant;
use runner_core::remote::{Computer, DeviceKind, PairedDevice, RemoteStatus, RemoteTrust};

pub(crate) use pairing::PairDialog;
pub(crate) use usage::usage_snapshot;
pub(crate) use workspace::{sidebar_nodes, Item as RemoteItem, RemoteWindow};

#[derive(Clone)]
struct PendingRemoteArchive {
    token: u64,
    computer: String,
    sessions: Vec<String>,
    missions: Vec<String>,
}

pub(crate) struct RemoteClient {
    pub(crate) ui: ClientUi,
    workspace: Option<RemoteWindow>,
    next_archive_token: u64,
    pending_archives: Vec<PendingRemoteArchive>,
    app_store: Entity<AppStore>,
    root_focus: FocusHandle,
    usage_popover: Option<usage::UsageWatch>,
    _subscription: Subscription,
}

impl RemoteClient {
    pub(crate) fn new(
        app_store: Entity<AppStore>,
        root_focus: FocusHandle,
        cx: &mut Context<NativeRoot>,
    ) -> Self {
        let clients = app_store.read(cx).remote_clients.clone();
        let subscription = cx.observe(&clients, |root, _, cx| root.sync_remote_state(cx));
        Self {
            ui: ClientUi::new(cx),
            workspace: None,
            next_archive_token: 0,
            pending_archives: Vec::new(),
            usage_popover: None,
            app_store,
            root_focus,
            _subscription: subscription,
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        self.workspace.is_some()
    }
    pub(crate) fn is_local(&self) -> bool {
        !self.is_active()
    }
    #[cfg(test)]
    pub(crate) fn workspace(&self) -> Option<&RemoteWindow> {
        self.workspace.as_ref()
    }
    #[cfg(test)]
    pub(crate) fn workspace_mut(&mut self) -> Option<&mut RemoteWindow> {
        self.workspace.as_mut()
    }
    pub(crate) fn disconnect(&mut self) {
        self.workspace = None;
        self.pending_archives.clear();
        self.usage_popover = None;
    }

    pub(crate) fn has_pending_archive(
        &self,
        sessions: &[String],
        missions: &[String],
        cx: &App,
    ) -> bool {
        let Some(remote) = self.workspace.as_ref() else {
            return false;
        };
        let computer = &remote.computer.id;
        let sidebar = remote.sidebar.read(cx);
        sessions.iter().any(|id| {
            sidebar.session_archiving(id)
                || self.pending_archives.iter().any(|entry| {
                    entry.computer == *computer && entry.sessions.iter().any(|item| item == id)
                })
        }) || missions.iter().any(|id| {
            self.pending_archives.iter().any(|entry| {
                entry.computer == *computer && entry.missions.iter().any(|item| item == id)
            })
        })
    }

    pub(crate) fn begin_pending_archive(
        &mut self,
        sessions: &[String],
        missions: &[String],
        cx: &mut Context<NativeRoot>,
    ) -> Option<u64> {
        if sessions.is_empty() || self.has_pending_archive(sessions, missions, cx) {
            return None;
        }
        let remote = self.workspace.as_mut()?;
        let computer = remote.computer.id.clone();
        let newly_marked_missions = {
            let sidebar = remote.sidebar.read(cx);
            missions
                .iter()
                .filter(|id| !sidebar.mission_archiving(id))
                .cloned()
                .collect::<Vec<_>>()
        };
        let token = self.next_archive_token;
        self.next_archive_token = self.next_archive_token.wrapping_add(1);
        self.pending_archives.push(PendingRemoteArchive {
            token,
            computer,
            sessions: sessions.to_vec(),
            missions: newly_marked_missions.clone(),
        });
        remote.sidebar.update(cx, |sidebar, cx| {
            sidebar.begin_archiving(sessions, &newly_marked_missions, cx);
        });
        Some(token)
    }

    pub(crate) fn finish_pending_archive(&mut self, token: u64, cx: &mut Context<NativeRoot>) {
        let Some(pos) = self
            .pending_archives
            .iter()
            .position(|entry| entry.token == token)
        else {
            return;
        };
        let removed = self.pending_archives.remove(pos);
        let clear_sessions = removed
            .sessions
            .into_iter()
            .filter(|id| {
                !self.pending_archives.iter().any(|entry| {
                    entry.computer == removed.computer
                        && entry.sessions.iter().any(|item| item == id)
                })
            })
            .collect::<Vec<_>>();
        let clear_missions = removed
            .missions
            .into_iter()
            .filter(|id| {
                !self.pending_archives.iter().any(|entry| {
                    entry.computer == removed.computer
                        && entry.missions.iter().any(|item| item == id)
                })
            })
            .collect::<Vec<_>>();
        if let Some(remote) = self
            .workspace
            .as_ref()
            .filter(|remote| remote.computer.id == removed.computer)
        {
            remote.sidebar.update(cx, |sidebar, cx| {
                sidebar.end_archiving(&clear_sessions, &clear_missions, cx);
            });
        }
    }

    pub(crate) fn clear_pending_archives_for(
        &mut self,
        computer: &str,
        cx: &mut Context<NativeRoot>,
    ) {
        let mut clear_sessions = Vec::new();
        let mut clear_missions = Vec::new();
        self.pending_archives.retain(|entry| {
            if entry.computer == computer {
                clear_sessions.extend(entry.sessions.iter().cloned());
                clear_missions.extend(entry.missions.iter().cloned());
                false
            } else {
                true
            }
        });
        if clear_sessions.is_empty() && clear_missions.is_empty() {
            return;
        }
        if let Some(remote) = self
            .workspace
            .as_ref()
            .filter(|remote| remote.computer.id == computer)
        {
            remote.sidebar.update(cx, |sidebar, cx| {
                sidebar.end_archiving(&clear_sessions, &clear_missions, cx);
            });
        }
    }

    fn restore_pending_archive_marks(
        &self,
        computer: &str,
        sidebar: &Entity<Sidebar>,
        cx: &mut Context<NativeRoot>,
    ) {
        let mut sessions = Vec::new();
        let mut missions = Vec::new();
        for entry in &self.pending_archives {
            if entry.computer == computer {
                sessions.extend(entry.sessions.iter().cloned());
                missions.extend(entry.missions.iter().cloned());
            }
        }
        if sessions.is_empty() && missions.is_empty() {
            return;
        }
        sidebar.update(cx, |sidebar, cx| {
            sidebar.begin_archiving(&sessions, &missions, cx);
        });
    }
    pub(crate) fn client(&self, cx: &App) -> crate::surfaces::front_page::FrontPageClient {
        self.workspace
            .as_ref()
            .map(|remote| remote.front_page.clone())
            .unwrap_or_else(|| {
                crate::surfaces::front_page::FrontPageClient::local(self.core(cx).clone())
            })
    }
    fn core<'a>(&self, cx: &'a App) -> &'a DaemonClient {
        &self.app_store.read(cx).client
    }
    fn settings<'a>(&self, cx: &'a App) -> &'a AppSettings {
        &self.app_store.read(cx).settings
    }
    fn terminal_style(&self, cx: &App) -> crate::terminal::element::TerminalStyle {
        let mut style = crate::surfaces::app_shell::terminal_style_for(self.settings(cx), cx);
        if let Some(palette) = self.remote_terminal_palette(cx) {
            style.palette = palette;
        }
        style
    }
}

#[derive(Default)]
pub(crate) struct RemoteSelection {
    active: Option<RemoteItem>,
    layouts: HashMap<String, PaneLayout>,
}

pub(crate) struct ClientUi {
    pub(crate) picker_open: bool,
    pub(crate) picker_signed_in: bool,
    pub(crate) picker_focus: FocusHandle,
    pub(crate) host_focus: FocusHandle,
    pub(crate) anchor: Option<Bounds<Pixels>>,
    pub(crate) footer_left: Option<Pixels>,
    pub(crate) pair: Option<PairDialog>,
    menus: std::cell::RefCell<HashMap<String, Entity<PopoverMenu>>>,
    pub(crate) pair_generation: u64,
    pub(crate) epoch: u64,
    pub(crate) selections: HashMap<String, RemoteSelection>,
}
impl ClientUi {
    pub(crate) fn new(cx: &mut Context<NativeRoot>) -> Self {
        Self {
            picker_focus: cx.focus_handle(),
            host_focus: cx.focus_handle(),
            picker_open: false,
            picker_signed_in: false,
            anchor: None,
            footer_left: None,
            pair: None,
            menus: Default::default(),
            pair_generation: 0,
            epoch: 0,
            selections: HashMap::new(),
        }
    }
}

pub(crate) fn label(key: &str, computer: &str) -> String {
    crate::i18n::tf(key, &[("computer", computer.into())])
}
fn icon(path: &'static str, color: gpui::Hsla) -> gpui::Svg {
    svg()
        .path(path)
        .size(rems(14. / 16.))
        .flex_none()
        .text_color(color)
}
fn status_label(computer: &Computer, connected: bool) -> String {
    if computer.is_self {
        return crate::i18n::text("client.access_self").into();
    }
    if connected {
        return crate::i18n::text("client.connected_here").into();
    }
    if computer.online {
        return crate::i18n::text("client.online").into();
    }
    computer
        .last_seen_at
        .as_ref()
        .map(|last| crate::i18n::tf("client.offline_seen", &[("time", seen_time(last))]))
        .unwrap_or_else(|| crate::i18n::text("client.offline").into())
}

fn device_status(device: &PairedDevice, connected: bool) -> String {
    if connected {
        return crate::i18n::text("client.connected_here").into();
    }
    let time = runner_app::text_util::format_relative_time(device.last_seen_at, chrono::Utc::now());
    crate::i18n::tf("client.offline_seen", &[("time", time)])
}

fn seen_time(value: &str) -> String {
    let Ok(time) = chrono::DateTime::parse_from_rfc3339(value) else {
        return value.into();
    };
    runner_app::text_util::format_relative_time(
        time.with_timezone(&chrono::Utc),
        chrono::Utc::now(),
    )
}

impl RemoteClient {
    pub(crate) fn refresh_computers(&self, cx: &mut Context<NativeRoot>) {
        state(&self.app_store, cx).update(cx, |state, cx| state.refresh(cx));
    }
    fn computer_menu(
        &self,
        computer: Option<&Computer>,
        devices: &[&PairedDevice],
        cx: &mut Context<NativeRoot>,
    ) -> Option<Entity<PopoverMenu>> {
        let incoming = devices
            .iter()
            .map(|device| device.fingerprint.clone())
            .collect::<Vec<_>>();
        let outgoing = computer.filter(|c| c.paired).map(|c| c.id.clone());
        if incoming.is_empty() && outgoing.is_none() {
            return None;
        }
        let key = computer
            .map(|c| c.id.clone())
            .unwrap_or_else(|| incoming.join("-"));
        let menu_key = format!("computer-menu-{key}");
        let owner = cx.weak_entity();
        // A device with no account computer has no name to tell it apart by but its fingerprint.
        let fingerprint = computer
            .is_none()
            .then(|| incoming.first().cloned())
            .flatten();
        let revoke_index = usize::from(fingerprint.is_some());
        let mut items = Vec::new();
        if let Some(fingerprint) = &fingerprint {
            items.push(
                UiMenuItem::new(crate::i18n::message("remote.fingerprint"))
                    .description(fingerprint.clone())
                    .header(),
            );
        }
        if !incoming.is_empty() {
            items.push(
                UiMenuItem::new(crate::i18n::message("remote.revoke"))
                    .icon("shield-x.svg")
                    .separator_before(fingerprint.is_some()),
            );
        }
        if outgoing.is_some() {
            items.push(UiMenuItem::new(crate::i18n::message("client.forget")).icon("unplug.svg"));
        }
        // Key by available actions too, so a successful pair refreshes the menu.
        let key = format!("{key}-{}-{}", incoming.join("-"), outgoing.is_some());
        Some(
            self.ui
                .menus
                .borrow_mut()
                .entry(key)
                .or_insert_with(|| {
                    cx.new(|cx| {
                        let menu = PopoverMenu::new(
                            menu_key,
                            cx.focus_handle(),
                            items,
                            Rc::new(move |index, _, cx| {
                                let _ = owner.update(cx, |root, cx| {
                                    if !incoming.is_empty() && index == revoke_index {
                                        let client = root.core(cx).clone();
                                        let fingerprints = incoming.clone();
                                        let store = root.app_store.clone();
                                        let task = cx.background_spawn(async move {
                                            for fingerprint in fingerprints {
                                                client.remote_revoke_device(fingerprint)?;
                                            }
                                            client.remote_trust()
                                        });
                                        cx.spawn(async move |weak, cx| {
                                            let result = task.await;
                                            let _ = weak.update(cx, |root, cx| match result {
                                                Ok(trust) => store.update(cx, |store, cx| {
                                                    store.remote_trust = Some(trust);
                                                    cx.notify();
                                                }),
                                                Err(_) => root.show_toast(
                                                    crate::i18n::text("client.action_failed"),
                                                    crate::toast::ToastTone::Error,
                                                    cx,
                                                ),
                                            });
                                        })
                                        .detach();
                                    } else if let Some(computer) = &outgoing {
                                        state(&root.app_store, cx)
                                            .read(cx)
                                            .send(Command::Forget(computer.clone()));
                                    }
                                });
                            }),
                            cx,
                        )
                        .trigger_icon("more-horizontal.svg")
                        .trigger_size(IconButtonSize::Sm);
                        if fingerprint.is_some() {
                            menu.min_width(px(240.))
                        } else {
                            menu
                        }
                    })
                })
                .clone(),
        )
    }

    pub(crate) fn render_computers(
        &self,
        trust: &RemoteTrust,
        status: &RemoteStatus,
        cx: &mut Context<NativeRoot>,
    ) -> AnyElement {
        let clients = state(&self.app_store, cx);
        let mut computers = clients.read(cx).computers.clone();
        if !computers.iter().any(|computer| computer.is_self) {
            computers.insert(0, model::this_computer(status, true));
        }
        let rows = computer_rows(&computers, trust);
        let count = rows.len();
        let this_name = computers
            .iter()
            .find(|computer| computer.is_self)
            .map(|computer| computer.name.clone())
            .unwrap_or_default();
        let mut table = div()
            .flex()
            .flex_col()
            .rounded_lg()
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .text_size(theme::text_meta())
                    .text_color(theme::faint())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(crate::i18n::text("remote.device")),
                    )
                    .child(
                        div()
                            .w(rems(200. / 16.))
                            .child(crate::i18n::text("client.status")),
                    )
                    .child(
                        div()
                            .w(rems(220. / 16.))
                            .child(crate::i18n::text("client.access")),
                    )
                    .child(div().w(rems(42. / 16.))),
            );
        for row in rows {
            let device = row.computer.is_none().then(|| row.devices[0]);
            let name = row
                .computer
                .map(|c| c.name.clone())
                .unwrap_or_else(|| model::device_title(row.devices[0]).to_owned());
            let connected = row.devices.iter().any(|d| d.connected);
            let menu = self.computer_menu(row.computer, &row.devices, cx);
            let mut action = div().w(rems(42. / 16.)).flex_none();
            if let Some(computer) = row
                .computer
                .filter(|c| !c.is_self && !c.paired && row.access == Access::Unpaired)
            {
                let computer = computer.clone();
                let root = cx.weak_entity();
                action = action.child(
                    Button::new(
                        SharedString::from(format!("pair-{}", computer.id)),
                        crate::i18n::text("client.pair"),
                    )
                    .size(ButtonSize::Sm)
                    .debug_selector("COMPUTER_PAIR_ACTION")
                    .on_press(move |window, cx| {
                        let _ = root.update(cx, |root, cx| {
                            root.remote_client.open_computer_pair(
                                computer.clone(),
                                pairing::Purpose::PairOnly,
                                window,
                                cx,
                            )
                        });
                    }),
                );
            } else if let Some(menu) = menu {
                action = action.child(menu);
            }
            let status = row
                .computer
                .map(|c| status_label(c, connected))
                .unwrap_or_else(|| device_status(row.devices[0], connected));
            table = table.child(
                div()
                    .debug_selector(|| "REMOTE_COMPUTER_ROW".into())
                    .when(row.computer.is_none(), |row| {
                        row.debug_selector(|| "REMOTE_DEVICE_ROW".into())
                    })
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .border_t_1()
                    .border_color(theme::border())
                    .text_size(theme::text_body())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                icon(
                                    device.map_or("monitor.svg", model::device_icon),
                                    theme::muted(),
                                )
                                .when(
                                    device.is_some_and(|d| d.kind == DeviceKind::Phone),
                                    |icon| icon.debug_selector(|| "REMOTE_PHONE_ICON".into()),
                                ),
                            )
                            .child(div().truncate().child(name))
                            .children((row.access == Access::This).then(|| {
                                div()
                                    .rounded_full()
                                    .bg(theme::raised())
                                    .px_1()
                                    .text_size(theme::text_micro())
                                    .text_color(theme::muted())
                                    .child(crate::i18n::text("client.this_computer"))
                            })),
                    )
                    .child(
                        div()
                            .w(rems(200. / 16.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_color(if connected {
                                theme::accent()
                            } else {
                                theme::muted()
                            })
                            .children((row.access != Access::This).then(|| {
                                div()
                                    .when(connected, |dot| {
                                        dot.debug_selector(|| "REMOTE_CONNECTED_NOW".into())
                                    })
                                    .size(rems(6. / 16.))
                                    .rounded_full()
                                    .bg(if connected || row.computer.is_some_and(|c| c.online) {
                                        theme::accent()
                                    } else {
                                        theme::faint()
                                    })
                            }))
                            .child(status),
                    )
                    .child(
                        div()
                            .w(rems(220. / 16.))
                            .text_color(if row.access == Access::Unpaired {
                                theme::faint()
                            } else {
                                theme::text()
                            })
                            .child(crate::i18n::text(row.access.key())),
                    )
                    .child(action),
            );
        }
        table = table.child(
            div()
                .border_t_1()
                .border_color(theme::border())
                .px_4()
                .py_3()
                .text_size(theme::text_meta())
                .text_color(theme::faint())
                .child(crate::i18n::tf(
                    "client.list_hint",
                    &[("computer", this_name)],
                )),
        );
        div()
            .debug_selector(|| "REMOTE_DEVICES".into())
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(crate::surfaces::settings::page::settings_section_heading(
                        crate::i18n::tf(
                            "remote.computers_heading",
                            &[(
                                "account",
                                crate::surfaces::settings::remote::account::username(status)
                                    .to_string(),
                            )],
                        ),
                    ))
                    .child(
                        div()
                            .rounded_full()
                            .bg(theme::raised())
                            .px_1()
                            .text_size(theme::text_micro())
                            .text_color(theme::muted())
                            .child(count.to_string()),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("refresh-computers", crate::i18n::text("ui.refresh"))
                            .debug_selector("REMOTE_COMPUTERS_REFRESH")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .icon("refresh-cw.svg")
                            .loading(clients.read(cx).loading)
                            .on_press({
                                let root = cx.weak_entity();
                                move |_, cx| {
                                    let _ = root.update(cx, |root, cx| {
                                        root.remote_client.refresh_computers(cx)
                                    });
                                }
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_4()
                    .text_size(theme::text_meta())
                    .text_color(theme::muted())
                    .children(
                        [Access::Incoming, Access::Outgoing, Access::Both]
                            .map(|access| div().child(crate::i18n::text(access.key()))),
                    ),
            )
            .when(clients.read(cx).action_failed, |body| {
                body.child(
                    div()
                        .text_size(theme::text_meta())
                        .text_color(theme::warning())
                        .child(crate::i18n::text("client.action_failed")),
                )
            })
            .when(clients.read(cx).directory_error, |body| {
                body.child(
                    div()
                        .debug_selector(|| "REMOTE_COMPUTERS_ERROR".into())
                        .text_size(theme::text_meta())
                        .text_color(theme::warning())
                        .child(crate::i18n::text("client.directory_failed")),
                )
            })
            .child(table)
            .into_any_element()
    }
}
