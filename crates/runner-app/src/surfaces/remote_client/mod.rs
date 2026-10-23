pub(crate) mod model;
mod pairing;
mod picker;
#[cfg(test)]
mod tests;
mod transport;
mod workspace;

use crate::*;
use gpui::{svg, FontWeight};
use model::{computer_rows, state, Access, Command, Connection};
use runner_app::ui::menu::popup_layer;
use runner_app::ui::ButtonVariant;
use runner_core::remote::{Computer, RemoteStatus, RemoteTrust};

pub(crate) use pairing::PairDialog;
pub(crate) use workspace::RemoteWindow;

pub(crate) struct ClientUi {
    pub(crate) picker_open: bool,
    pub(crate) picker_focus: FocusHandle,
    pub(crate) anchor: Option<Bounds<Pixels>>,
    pub(crate) pair: Option<PairDialog>,
    menus: std::cell::RefCell<HashMap<String, Entity<PopoverMenu>>>,
    pub(crate) pair_generation: u64,
    pub(crate) epoch: u64,
}
impl ClientUi {
    pub(crate) fn new(cx: &mut Context<NativeRoot>) -> Self {
        Self {
            picker_focus: cx.focus_handle(),
            picker_open: false,
            anchor: None,
            pair: None,
            menus: Default::default(),
            pair_generation: 0,
            epoch: 0,
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

fn seen_time(value: &str) -> String {
    let Ok(time) = chrono::DateTime::parse_from_rfc3339(value) else {
        return value.into();
    };
    let minutes = chrono::Utc::now()
        .signed_duration_since(time)
        .num_minutes()
        .max(0) as usize;
    if minutes == 0 {
        crate::i18n::text("ui.just_now").into()
    } else if minutes < 60 {
        crate::i18n::tnf("format.minute_ago", minutes, &[])
    } else if minutes < 1440 {
        crate::i18n::tnf("format.hour_ago", minutes / 60, &[])
    } else {
        crate::i18n::tnf("format.day_ago", minutes / 1440, &[])
    }
}

impl NativeRoot {
    pub(crate) fn refresh_computers(&self, cx: &mut Context<Self>) {
        state(&self.app_store, cx).update(cx, |state, cx| state.refresh(cx));
    }
    fn computer_menu(
        &self,
        computer: Option<&Computer>,
        devices: &[&runner_core::remote::PairedDevice],
        cx: &mut Context<Self>,
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
        let mut items = Vec::new();
        if !incoming.is_empty() {
            items.push(UiMenuItem::new(crate::i18n::text("remote.revoke")).icon("shield-x.svg"));
        }
        if outgoing.is_some() {
            items.push(UiMenuItem::new(crate::i18n::text("client.forget")).icon("unplug.svg"));
        }
        // Key by available actions too, so a successful pair refreshes the menu.
        let key = format!("{key}-{}-{}", incoming.join("-"), outgoing.is_some());
        Some(
            self.client_ui
                .menus
                .borrow_mut()
                .entry(key)
                .or_insert_with(|| {
                    cx.new(|cx| {
                        PopoverMenu::new(
                            menu_key,
                            cx.focus_handle(),
                            items,
                            Rc::new(move |index, _, cx| {
                                let _ = owner.update(cx, |root, cx| {
                                    if !incoming.is_empty() && index == 0 {
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
                        .trigger_size(IconButtonSize::Sm)
                    })
                })
                .clone(),
        )
    }

    pub(crate) fn render_computers(
        &self,
        trust: &RemoteTrust,
        status: &RemoteStatus,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let clients = state(&self.app_store, cx);
        let mut computers = clients.read(cx).computers.clone();
        if !computers.iter().any(|computer| computer.is_self) {
            computers.insert(
                0,
                Computer {
                    id: status.host_id.clone().unwrap_or_default(),
                    name: status
                        .host_name
                        .clone()
                        .unwrap_or_else(|| crate::i18n::text("client.this_computer").into()),
                    is_self: true,
                    ..Default::default()
                },
            );
        }
        let rows = computer_rows(&computers, trust);
        let count = rows.len();
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
                            .child(crate::i18n::text("remote.computer")),
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
            let name = row
                .computer
                .map(|c| c.name.clone())
                .unwrap_or_else(|| row.devices[0].name.clone());
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
                    .on_press(move |window, cx| {
                        let _ = root.update(cx, |root, cx| {
                            root.open_computer_pair(
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
                .unwrap_or_else(|| {
                    if connected {
                        crate::i18n::text("client.connected_here").into()
                    } else {
                        row.devices[0]
                            .last_seen_at
                            .with_timezone(&chrono::Local)
                            .format("%Y-%m-%d %H:%M")
                            .to_string()
                    }
                });
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
                            .child(icon("monitor.svg", theme::muted()))
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
                .child(crate::i18n::text("client.list_hint")),
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
                    .child(crate::surfaces::settings_page::settings_section_heading(
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
                        .text_size(theme::text_meta())
                        .text_color(theme::warning())
                        .child(crate::i18n::text("client.directory_failed")),
                )
            })
            .child(table)
            .into_any_element()
    }
}
