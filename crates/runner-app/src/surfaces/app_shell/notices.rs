use super::*;

impl NativeRoot {
    pub(crate) fn render_daemon_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let store = self.app_store.read(cx);
        if !store.daemon_disconnected {
            return None;
        }
        let message = match store.daemon_notice {
            Some(runner_app::lifecycle::DaemonNotice::Repeated) => {
                runner_core::protocol::managed::RESTART_LIMIT_NOTICE
            }
            Some(runner_app::lifecycle::DaemonNotice::InUse) => {
                runner_core::protocol::managed::IN_USE_NOTICE
            }
            _ => return None,
        };
        let root = cx.weak_entity();
        let log_path = self.log_dir.join("runnerd.log");
        Some(
            runner_app::ui::notice_banner(
                div()
                    .debug_selector(|| "DAEMON_CRASH_MESSAGE".into())
                    .truncate()
                    .child(message),
                runner_app::ui::Tone::Danger,
            )
            .id("daemon-crash-banner")
            .debug_selector(|| "DAEMON_CRASH_BANNER".into())
            .child(
                div()
                    .flex_none()
                    .debug_selector(|| "DAEMON_OPEN_LOG".into())
                    .child(
                        Button::new("daemon-open-log", crate::i18n::text("ui.open_log"))
                            .size(ButtonSize::Sm)
                            .variant(runner_app::ui::ButtonVariant::Ghost)
                            .on_press(move |_, cx| cx.reveal_path(&log_path)),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .debug_selector(|| "DAEMON_TRY_AGAIN".into())
                    .child(
                        Button::new("daemon-try-again", crate::i18n::text("ui.try_again"))
                            .size(ButtonSize::Sm)
                            .radius(8.)
                            .variant(runner_app::ui::ButtonVariant::Secondary)
                            .disabled(self.daemon_retrying)
                            .loading(self.daemon_retrying)
                            .on_press(move |window, cx| {
                                let _ = root.update(cx, |this, cx| this.retry_daemon(window, cx));
                            }),
                    ),
            )
            .into_any_element(),
        )
    }

    pub(super) fn render_toast(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.toasts.active().map(|toast| {
            let zoom = self.settings(cx).app_zoom;
            let icon = toast.icon.unwrap_or_else(|| toast.tone.icon());
            let icon_color = match toast.tone {
                ToastTone::Info => theme::muted(),
                ToastTone::Success | ToastTone::Restart => theme::accent(),
                ToastTone::Warning => theme::warning(),
                ToastTone::Error => theme::danger(),
            };
            div()
                .absolute()
                .when(toast.bottom, |toast| toast.bottom(px(24. * zoom)))
                .when(!toast.bottom, |toast| {
                    toast.top(px((TITLEBAR_DRAG_HEIGHT + 12.) * zoom))
                })
                .left(px(16. * zoom))
                .right(px(16. * zoom))
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("global-toast")
                        .debug_selector(|| "GLOBAL_TOAST".into())
                        .max_w(px(if toast.single_line { 600. } else { 420. } * zoom))
                        .pl(px(14. * zoom))
                        .pr(px(12. * zoom))
                        .py(px(if toast.open_remote { 8. } else { 10. } * zoom))
                        .flex()
                        .items_center()
                        .gap(px(10. * zoom))
                        .rounded(px(10. * zoom))
                        .border_1()
                        .border_color(theme::border())
                        .bg(theme::panel())
                        .shadow(vec![BoxShadow {
                            color: theme::scrim(),
                            offset: point(px(0.), px(8. * zoom)),
                            blur_radius: px(24. * zoom),
                            spread_radius: px(0.),
                            inset: false,
                        }])
                        .cursor_pointer()
                        .occlude()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.toasts.dismiss();
                            cx.notify();
                        }))
                        .child(
                            svg()
                                .flex_none()
                                .path(icon)
                                .w(px(16. * zoom))
                                .h(px(16. * zoom))
                                .text_color(icon_color),
                        )
                        .child(
                            div()
                                .debug_selector(|| "GLOBAL_TOAST_MESSAGE".into())
                                .min_w(px(0.))
                                .whitespace_normal()
                                .when(toast.single_line, |message| message.truncate())
                                .text_size(theme::text_body())
                                .text_color(theme::text())
                                .line_height(px(18. * zoom))
                                .child(SharedString::from(toast.message.clone())),
                        )
                        .children(toast.open_remote.then(|| {
                            div()
                                .id("toast-open-remote")
                                .debug_selector(|| "REMOTE_TOAST_ACTION".into())
                                .tab_index(0)
                                .text_size(theme::text_body())
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(crate::i18n::text("remote.open_remote"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.toasts.dismiss();
                                    this.enter_settings_route(Some("remote"), window, cx);
                                }))
                                .on_key_down(cx.listener(
                                    |this, event: &KeyDownEvent, window, cx| {
                                        if matches!(event.keystroke.key.as_str(), "enter" | "space")
                                        {
                                            cx.stop_propagation();
                                            this.toasts.dismiss();
                                            this.enter_settings_route(Some("remote"), window, cx);
                                        }
                                    },
                                ))
                        }))
                        .child(
                            svg()
                                .debug_selector(|| "GLOBAL_TOAST_CLOSE".into())
                                .flex_none()
                                .ml(px(6. * zoom))
                                .path("close.svg")
                                .w(px(14. * zoom))
                                .h(px(14. * zoom))
                                .text_color(theme::faint())
                                .hover(|close| close.text_color(theme::text())),
                        ),
                )
                .into_any_element()
        })
    }

    pub(crate) fn show_toast(
        &mut self,
        message: impl Into<runner_app::i18n::Text>,
        tone: ToastTone,
        cx: &mut Context<Self>,
    ) {
        if self.app_store.read(cx).daemon_disconnected {
            return;
        }
        let id = self.toasts.show(message, tone);
        let duration_ms = self.toasts.active().and_then(|toast| toast.duration_ms);
        cx.notify();
        let Some(duration_ms) = duration_ms else {
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(duration_ms))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.toasts.expire(id) {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn retry_daemon(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.daemon_retrying {
            return;
        }
        self.daemon_retrying = true;
        self.mission_workspace.update(cx, |_, cx| cx.notify());
        let client = self.core(cx).clone();
        let recovery = cx.background_spawn(async move { client.reconnect() });
        cx.spawn_in(window, async move |weak, cx| {
            let result = recovery.await;
            let _ = weak.update_in(cx, |this, _, cx| {
                this.daemon_retrying = false;
                this.mission_workspace.update(cx, |_, cx| cx.notify());
                if let Err(error) = result {
                    tracing::warn!("runnerd manual recovery failed: {error}");
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
