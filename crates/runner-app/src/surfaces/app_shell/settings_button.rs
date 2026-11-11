use super::*;

impl NativeRoot {
    pub(crate) fn render_settings_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let zoom = self.settings(cx).app_zoom;
        let update = update_dot_visible(&self.agent_updates, self.settings(cx));
        div()
            .id("sidebar-settings")
            .debug_selector(|| "SIDEBAR_SETTINGS".into())
            .tab_index(0)
            .relative()
            .flex_none()
            .size(px(32. * zoom))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .cursor_pointer()
            .occlude()
            .hover(|button| button.bg(theme::chrome_hover()))
            .focus_visible(|button| button.bg(theme::chrome_hover()))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |root, _, window, cx| {
                root.open_this_computer_settings(update.then_some("agents"), window, cx)
            }))
            .on_key_down(cx.listener(move |root, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    root.open_this_computer_settings(update.then_some("agents"), window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                svg()
                    .path("settings.svg")
                    .size(px(16. * zoom))
                    .text_color(theme::muted()),
            )
            .children(update.then(|| {
                div()
                    .debug_selector(|| "SIDEBAR_SETTINGS_UPDATE_DOT".into())
                    .absolute()
                    .top(px(5. * zoom))
                    .right(px(5. * zoom))
                    .size(px(6. * zoom))
                    .rounded_full()
                    .bg(theme::accent())
            }))
            .into_any_element()
    }
}

#[cfg(test)]
mod remote_isolation_tests {
    use crate::surfaces::remote_client::tests::{
        add_window, choose_host, enable_front_page, fixture,
    };
    use gpui::{TestAppContext, VisualTestContext};

    #[test]
    fn remote_sidebar_keeps_settings_for_this_computer_and_refuses_stale_sign_out() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = fixture(&mut cx, temp.path());
        enable_front_page(&fake);
        let window = add_window(&mut cx, &store, temp.path(), "remote-footer");
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.run_until_parked();
        assert!(visual.debug_bounds("SIDEBAR_SETTINGS").is_some());
        window
            .update(&mut visual, |root, window, cx| {
                choose_host(root, window, cx)
            })
            .unwrap();
        visual.run_until_parked();
        let gear = visual.debug_bounds("SIDEBAR_SETTINGS").unwrap();
        let before = fake.local_call_count();
        window
            .update(&mut visual, |root, _, cx| {
                root.confirm_sign_out(cx);
                assert!(!root.settings_page.remote.confirm);
            })
            .unwrap();
        visual.run_until_parked();
        assert_eq!(fake.local_call_count(), before);
        visual.simulate_click(gear.center(), Default::default());
        visual.run_until_parked();
        window
            .read_with(&visual, |root, _| {
                assert!(root.remote_client.is_local());
                assert_eq!(root.route, crate::AppRoute::Settings);
            })
            .unwrap();
    }
}
