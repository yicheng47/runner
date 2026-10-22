use super::*;
use crate::surfaces::settings::remote::account::{account_initial, avatar, signed_in};
use runner_app::ui::menu::{
    popup_layer_positioned, MenuAction, MenuItem, MenuKey, MenuState, PopupWidth,
};
use runner_core::remote::RemoteStatus;

pub(crate) struct AccountMenuState {
    pub(crate) state: MenuState,
    anchor: Option<Bounds<Pixels>>,
    focus: FocusHandle,
    return_focus: Option<FocusHandle>,
}
impl AccountMenuState {
    pub(crate) fn new(cx: &mut Context<NativeRoot>) -> Self {
        Self {
            state: MenuState::default(),
            anchor: None,
            focus: cx.focus_handle(),
            return_focus: None,
        }
    }
}

fn items(status: &RemoteStatus) -> Vec<MenuItem> {
    let mut items = Vec::new();
    if signed_in(status) {
        items.push(MenuItem::new(crate::i18n::text("account.title")).icon("user-round.svg"));
    }
    items.push(
        MenuItem::new(crate::i18n::text("ui.settings"))
            .icon("settings.svg")
            .shortcut(if cfg!(target_os = "macos") {
                "⌘,"
            } else {
                "Ctrl+,"
            }),
    );
    items.push(
        MenuItem::new(crate::i18n::text(if signed_in(status) {
            "account.sign_out_menu"
        } else {
            "account.sign_in_menu"
        }))
        .icon(if signed_in(status) {
            "log-out.svg"
        } else {
            "log-in.svg"
        })
        .separator_before(true),
    );
    items
}

impl NativeRoot {
    fn toggle_account_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.account_menu.state.open {
            self.close_account_menu(window, cx);
        } else {
            let status = self
                .app_store
                .read(cx)
                .remote_status
                .clone()
                .unwrap_or_default();
            self.account_menu.state.open(&items(&status), 0);
            self.account_menu.return_focus = window.focused(cx);
            self.account_menu.focus.focus(window, cx);
            self.usage_open = false;
            cx.notify();
        }
    }

    fn close_account_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.account_menu.state.close();
        if let Some(focus) = self.account_menu.return_focus.take() {
            focus.focus(window, cx);
        }
        cx.notify();
    }

    fn activate_account_menu(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let status = self
            .app_store
            .read(cx)
            .remote_status
            .clone()
            .unwrap_or_default();
        let signed_in = signed_in(&status);
        self.close_account_menu(window, cx);
        if signed_in && index == 0 {
            self.enter_settings_route(Some("account"), window, cx);
        } else if index == usize::from(signed_in) {
            self.enter_settings_route(None, window, cx);
        } else {
            self.enter_settings_route(Some("account"), window, cx);
            if signed_in {
                self.confirm_sign_out(cx);
                self.settings_page.remote.focus.focus(window, cx);
            }
        }
    }

    pub(super) fn render_account_avatar(&self, cx: &mut Context<Self>) -> AnyElement {
        let zoom = self.settings(cx).app_zoom;
        let status = self
            .app_store
            .read(cx)
            .remote_status
            .clone()
            .unwrap_or_default();
        let owner = cx.entity();
        div()
            .id("account-avatar")
            .debug_selector(|| "SIDEBAR_ACCOUNT".into())
            .tab_index(0)
            .flex_none()
            .size(px(32. * zoom))
            .flex()
            .items_center()
            .justify_center()
            .relative()
            .rounded_full()
            .cursor_pointer()
            .occlude()
            .hover(|button| button.bg(theme::chrome_hover()))
            .focus_visible(|button| button.bg(theme::chrome_hover()))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| this.toggle_account_menu(window, cx)))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space" | "down") {
                    this.toggle_account_menu(window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .debug_selector(move || {
                        if account_initial(&status).is_some() {
                            "SIDEBAR_ACCOUNT_INITIAL".into()
                        } else {
                            "SIDEBAR_ACCOUNT_ICON".into()
                        }
                    })
                    .size(px(28. * zoom))
                    .rounded_full()
                    .border_1()
                    .border_color(theme::chrome_selected_border())
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(rems(11. / 16.))
                    .child(avatar(
                        &self
                            .app_store
                            .read(cx)
                            .remote_status
                            .clone()
                            .unwrap_or_default(),
                        26.,
                    )),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, _, cx| {
                        owner.update(cx, |this, _| this.account_menu.anchor = Some(bounds));
                    },
                )
                .absolute()
                .inset_0(),
            )
            .into_any_element()
    }

    pub(super) fn render_account_menu(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.account_menu.state.open {
            return None;
        }
        let anchor = self.account_menu.anchor?;
        let status = self
            .app_store
            .read(cx)
            .remote_status
            .clone()
            .unwrap_or_default();
        let signed_in = signed_in(&status);
        let (name, description) = if signed_in {
            let account = status.account.as_ref();
            (
                account.map(|a| a.display_name.clone()).unwrap_or_default(),
                format!(
                    "@{} · {}",
                    account.map(|a| a.username.as_str()).unwrap_or_default(),
                    status.origin
                ),
            )
        } else {
            (
                crate::i18n::text("account.signed_out").into(),
                crate::i18n::text("account.menu_beta").into(),
            )
        };
        let menu_items = items(&status);
        let mut menu = div()
            .id("account-menu")
            .debug_selector(|| "ACCOUNT_MENU".into())
            .track_focus(&self.account_menu.focus)
            .rounded(rems(10. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .p_1()
            .text_size(theme::text_body())
            .text_color(theme::text())
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let key = match event.keystroke.key.as_str() {
                    "up" => MenuKey::Up,
                    "down" | "tab" => {
                        if event.keystroke.modifiers.shift {
                            MenuKey::Up
                        } else {
                            MenuKey::Down
                        }
                    }
                    "home" => MenuKey::Home,
                    "end" => MenuKey::End,
                    "enter" | "space" => MenuKey::Enter,
                    "escape" => MenuKey::Escape,
                    _ => return,
                };
                let status = this
                    .app_store
                    .read(cx)
                    .remote_status
                    .clone()
                    .unwrap_or_default();
                match this.account_menu.state.handle_key(key, &items(&status)) {
                    MenuAction::Activate(index) => this.activate_account_menu(index, window, cx),
                    MenuAction::Close => this.close_account_menu(window, cx),
                    MenuAction::None => cx.notify(),
                }
                cx.stop_propagation();
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .p_2()
                    .pb_3()
                    .child(avatar(&status, 32.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().truncate().font_weight(FontWeight::MEDIUM).child(name))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(theme::text_meta())
                                    .text_color(theme::faint())
                                    .child(description),
                            ),
                    ),
            );
        for (index, item) in menu_items.into_iter().enumerate() {
            if index == 0 || item.separator_before {
                menu = menu.child(div().h(px(1.)).bg(theme::border()).my_1());
            }
            menu = menu.child(
                div()
                    .id(("account-menu-item", index))
                    .debug_selector(move || format!("ACCOUNT_MENU_ITEM_{index}"))
                    .h(rems(32. / 16.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(rems(6. / 16.))
                    .cursor_pointer()
                    .when(self.account_menu.state.highlighted == index, |row| {
                        row.bg(theme::raised())
                    })
                    .hover(|row| row.bg(theme::raised()))
                    .child(
                        svg()
                            .path(item.icon.unwrap())
                            .size(rems(14. / 16.))
                            .text_color(theme::muted()),
                    )
                    .child(div().flex_1().child(item.label))
                    .children(item.shortcut.map(|shortcut| {
                        div()
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .child(shortcut)
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_account_menu(index, window, cx)
                    })),
            );
        }
        let root = cx.entity();
        Some(popup_layer_positioned(
            anchor,
            window,
            PopupWidth::Fixed(px(260. * self.settings(cx).app_zoom)),
            px(12. * self.settings(cx).app_zoom),
            menu.into_any_element(),
            Rc::new(move |window, cx| {
                root.update(cx, |this, cx| this.close_account_menu(window, cx));
            }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{size, TestAppContext, VisualTestContext};
    use runner_app::i18n::{testing::with_locale, Locale};
    use runner_core::remote::{RemoteAccount, RemoteState, StopReason};

    #[test]
    fn avatar_initial_and_menu_items_follow_sign_in_including_stopped_credentials() {
        let mut status = RemoteStatus {
            account: Some(RemoteAccount {
                username: "jason".into(),
                display_name: "  Jason".into(),
            }),
            host_id: Some("host".into()),
            ..Default::default()
        };
        for state in [
            RemoteState::Off,
            RemoteState::Stopped {
                reason: StopReason::InUseElsewhere,
            },
            RemoteState::Stopped {
                reason: StopReason::Removed,
            },
        ] {
            status.state = state;
            assert_eq!(account_initial(&status), None);
            assert_eq!(items(&status).len(), 2);
        }
        for state in [
            RemoteState::Connecting { attempt: 0 },
            RemoteState::Connected {
                since: chrono::Utc::now(),
            },
            RemoteState::Paused,
            RemoteState::Reconnecting {
                attempt: 1,
                next_retry_at: chrono::Utc::now(),
            },
        ] {
            status.state = state;
            assert_eq!(account_initial(&status), Some("J".into()));
            with_locale(Locale::English, || {
                let labels = items(&status)
                    .into_iter()
                    .map(|item| item.label.to_string())
                    .collect::<Vec<_>>();
                assert_eq!(labels, ["Account", "Settings", "Sign out…"]);
            });
        }
        status.account.as_mut().unwrap().display_name = "王".into();
        assert_eq!(account_initial(&status), Some("王".into()));
        status.state = RemoteState::Off;
        with_locale(Locale::English, || {
            let labels = items(&status)
                .into_iter()
                .map(|item| item.label.to_string())
                .collect::<Vec<_>>();
            assert_eq!(labels, ["Settings", "Sign in…"]);
        });
    }

    #[test]
    fn menu_opens_above_avatar_and_opens_account_and_sign_out_confirmation() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for locale in [Locale::English, Locale::SimplifiedChinese] {
            with_locale(locale, || {
                let temp = tempfile::tempdir().unwrap();
                let mut cx = TestAppContext::single();
                let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
                let host = cx.add_window(|window, cx| {
                    NativeRoot::new(
                        "avatar-menu".into(),
                        temp.path().join("logs"),
                        None,
                        None,
                        store.clone(),
                        window,
                        cx,
                    )
                });
                let mut visual = VisualTestContext::from_window(host.into(), &cx);
                visual.simulate_resize(size(px(1200.), px(900.)));
                visual.run_until_parked();
                for signed in [false, true] {
                    store.update(&mut visual, |store, cx| {
                        store.remote_status = Some(RemoteStatus {
                            state: if signed {
                                RemoteState::Paused
                            } else {
                                RemoteState::Off
                            },
                            account: Some(RemoteAccount {
                                username: "jason".into(),
                                display_name: "Jason".into(),
                            }),
                            ..Default::default()
                        });
                        cx.notify();
                    });
                    host.update(&mut visual, |root, _, cx| {
                        root.route = AppRoute::Chat;
                        root.dismiss_remote_dialog();
                        cx.notify();
                    })
                    .unwrap();
                    visual.run_until_parked();
                    let avatar = visual.debug_bounds("SIDEBAR_ACCOUNT").unwrap();
                    assert_eq!(
                        visual.debug_bounds("SIDEBAR_ACCOUNT_INITIAL").is_some(),
                        signed
                    );
                    assert_eq!(
                        visual.debug_bounds("SIDEBAR_ACCOUNT_ICON").is_some(),
                        !signed
                    );
                    visual.simulate_click(avatar.center(), Default::default());
                    visual.run_until_parked();
                    let menu = visual.debug_bounds("ACCOUNT_MENU").unwrap();
                    assert!(menu.bottom() < avatar.top());
                    assert_eq!(menu.size.width, px(260.));
                    assert_eq!(visual.debug_bounds("ACCOUNT_MENU_ITEM_2").is_some(), signed);
                    visual.simulate_click(avatar.center(), Default::default());
                    visual.run_until_parked();
                    assert!(visual.debug_bounds("ACCOUNT_MENU").is_none());
                    visual.simulate_click(avatar.center(), Default::default());
                    visual.run_until_parked();
                    assert!(visual.debug_bounds("ACCOUNT_MENU").is_some());
                    visual.simulate_keystrokes("escape");
                    visual.run_until_parked();
                    assert!(visual.debug_bounds("ACCOUNT_MENU").is_none());
                    if signed {
                        visual.simulate_click(avatar.center(), Default::default());
                        visual.run_until_parked();
                        let account = visual.debug_bounds("ACCOUNT_MENU_ITEM_0").unwrap();
                        visual.simulate_click(account.center(), Default::default());
                        visual.run_until_parked();
                        assert!(visual.debug_bounds("ACCOUNT_STATUS_CARD").is_some());
                        host.update(&mut visual, |root, _, cx| {
                            root.route = AppRoute::Chat;
                            cx.notify();
                        })
                        .unwrap();
                        visual.run_until_parked();
                        let avatar = visual.debug_bounds("SIDEBAR_ACCOUNT").unwrap();
                        visual.simulate_click(avatar.center(), Default::default());
                        visual.run_until_parked();
                        visual.simulate_keystrokes("end enter");
                        visual.run_until_parked();
                        assert!(visual.debug_bounds("CONFIRM_DIALOG_PANEL").is_some());
                        visual.simulate_keystrokes("escape");
                        visual.run_until_parked();
                        assert!(visual.debug_bounds("CONFIRM_DIALOG_PANEL").is_none());
                    }
                }
            });
        }
    }
}
