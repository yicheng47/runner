use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, Context, CursorStyle, Entity, FontWeight, KeyDownEvent, Render,
    Subscription, Window,
};
use runner_app::ui::{Button, ButtonSize, ButtonVariant, PaneHeader, SettingsCard, Toggle};
use runner_core::protocol::{
    TelemetryStatus, DEFAULT_HEARTBEAT_ENDPOINT, PRIVACY_GUIDE_URL_EN, PRIVACY_GUIDE_URL_ZH,
};

use crate::app_store::AppStore;
use crate::surfaces::settings::page::settings_section_heading;
use crate::theme;

pub(crate) fn privacy_guide_url() -> &'static str {
    match runner_app::i18n::active_locale() {
        runner_app::i18n::Locale::SimplifiedChinese => PRIVACY_GUIDE_URL_ZH,
        runner_app::i18n::Locale::English => PRIVACY_GUIDE_URL_EN,
    }
}

pub(crate) struct PrivacyPane {
    app_store: Entity<AppStore>,
    busy: bool,
    read_generation: u64,
    mutation_generation: u64,
    _subscriptions: Vec<Subscription>,
}

impl PrivacyPane {
    pub(crate) fn new(app_store: Entity<AppStore>, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.observe(&app_store, |_, _, cx| cx.notify())];
        Self {
            app_store,
            busy: false,
            read_generation: 0,
            mutation_generation: 0,
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        // Defer refresh while a mutation is in flight: the mutation itself returns
        // the authoritative updated TelemetryStatus and owns clearing `busy`.
        if self.busy {
            return;
        }
        self.read_generation = self.read_generation.wrapping_add(1);
        let generation = self.read_generation;
        let client = self.app_store.read(cx).client.clone();
        let bg = cx.background_spawn(async move { client.telemetry_status().ok() });
        cx.spawn(async move |weak, cx| {
            let status = bg.await;
            let _ = weak.update(cx, |this, cx| {
                if this.busy || this.read_generation != generation {
                    return;
                }
                if let Some(status) = status {
                    this.app_store.update(cx, |store, store_cx| {
                        store.set_telemetry_status(status, store_cx)
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.read_generation = self.read_generation.wrapping_add(1);
        self.mutation_generation = self.mutation_generation.wrapping_add(1);
        let generation = self.mutation_generation;
        let client = self.app_store.read(cx).client.clone();
        cx.notify();
        let bg = cx.background_spawn(async move { client.telemetry_set_enabled(enabled).ok() });
        cx.spawn(async move |weak, cx| {
            let status = bg.await;
            let _ = weak.update(cx, |this, cx| {
                if this.mutation_generation != generation {
                    return;
                }
                this.busy = false;
                if let Some(status) = status {
                    this.app_store.update(cx, |store, store_cx| {
                        store.set_telemetry_status(status, store_cx)
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn reset_id(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.read_generation = self.read_generation.wrapping_add(1);
        self.mutation_generation = self.mutation_generation.wrapping_add(1);
        let generation = self.mutation_generation;
        let client = self.app_store.read(cx).client.clone();
        cx.notify();
        let bg = cx.background_spawn(async move { client.telemetry_reset_id().ok() });
        cx.spawn(async move |weak, cx| {
            let status = bg.await;
            let _ = weak.update(cx, |this, cx| {
                if this.mutation_generation != generation {
                    return;
                }
                this.busy = false;
                if let Some(status) = status {
                    this.app_store.update(cx, |store, store_cx| {
                        store.set_telemetry_status(status, store_cx)
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn status(&self, cx: &Context<Self>) -> TelemetryStatus {
        self.app_store
            .read(cx)
            .telemetry
            .clone()
            .unwrap_or_else(|| TelemetryStatus {
                enabled: true,
                stored_enabled: true,
                env_override: None,
                dev_build: runner_core::version::is_dev_build(),
                install_id: None,
                payload_preview: "{}".to_owned(),
                endpoint: DEFAULT_HEARTBEAT_ENDPOINT.to_owned(),
                last_sent_at: None,
            })
    }

    fn render_heartbeat_row(
        &self,
        status: &TelemetryStatus,
        loaded: bool,
        effective_enabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let pane = cx.entity();
        let env_override = status.env_override.clone();
        let toggle_disabled = !loaded || env_override.is_some() || self.busy;
        let guide_url = privacy_guide_url();
        let mut meta_row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(rems(8. / 16.))
            .gap_y(rems(4. / 16.))
            .pt(rems(2. / 16.))
            .child(
                div()
                    .id("privacy-guide-link")
                    .debug_selector(|| "PRIVACY_GUIDE_LINK".into())
                    .tab_index(0)
                    .cursor(CursorStyle::PointingHand)
                    .flex()
                    .items_center()
                    .gap(rems(3. / 16.))
                    .text_size(theme::text_meta())
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::accent())
                    .hover(|link| link.opacity(0.85))
                    .on_click(move |_, _, cx| cx.open_url(guide_url))
                    .on_key_down(move |event: &KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            cx.open_url(guide_url);
                        }
                    })
                    .child(crate::i18n::text("privacy.privacy_guide"))
                    .child(
                        svg()
                            .flex_none()
                            .path("arrow-up-right.svg")
                            .size(rems(11. / 16.))
                            .text_color(theme::accent()),
                    ),
            );

        if env_override.is_none() {
            meta_row = meta_row
                .child(
                    div()
                        .text_size(theme::text_meta())
                        .text_color(theme::faint())
                        .child("·"),
                )
                .child(
                    div()
                        .debug_selector(|| "PRIVACY_STATUS_CAPTION".into())
                        .text_size(theme::text_meta())
                        .text_color(theme::faint())
                        .child(if effective_enabled {
                            crate::i18n::text("privacy.honors_dnt")
                        } else {
                            crate::i18n::text("privacy.paused_caption")
                        }),
                );
        }

        div()
            .flex()
            .flex_col()
            .gap(rems(10. / 16.))
            .px_4()
            .py_3()
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap(rems(20. / 16.))
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(rems(4. / 16.))
                            .child(
                                div()
                                    .text_size(theme::text_body())
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::text())
                                    .child(crate::i18n::text("privacy.heartbeat_label")),
                            )
                            .child(
                                div()
                                    .whitespace_normal()
                                    .text_size(theme::text_meta())
                                    .line_height(rems(17. / 16.))
                                    .text_color(theme::muted())
                                    .child(crate::i18n::text("privacy.heartbeat_description")),
                            )
                            .child(meta_row),
                    )
                    .child(
                        div()
                            .flex_none()
                            .pt(rems(2. / 16.))
                            .debug_selector(|| "PRIVACY_HEARTBEAT_TOGGLE".into())
                            .child(
                                Toggle::new("privacy-heartbeat-toggle", effective_enabled)
                                    .disabled(toggle_disabled)
                                    .on_change(move |enabled, _, cx| {
                                        pane.update(cx, |pane, pane_cx| {
                                            pane.set_enabled(enabled, pane_cx);
                                        });
                                    }),
                            ),
                    ),
            )
            .children(env_override.map(|var| {
                div()
                    .debug_selector(|| "PRIVACY_ENV_NOTICE".into())
                    .flex()
                    .items_center()
                    .gap(rems(6. / 16.))
                    .rounded(rems(6. / 16.))
                    .border_1()
                    .border_color(theme::with_alpha(theme::warning(), 0.3))
                    .bg(theme::with_alpha(theme::warning(), 0.12))
                    .px(rems(10. / 16.))
                    .py(rems(6. / 16.))
                    .child(
                        svg()
                            .flex_none()
                            .path("terminal.svg")
                            .size(rems(12. / 16.))
                            .text_color(theme::warning()),
                    )
                    .child(
                        div()
                            .min_w(px(0.))
                            .whitespace_normal()
                            .text_size(theme::text_meta())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::warning())
                            .child(crate::i18n::tf(
                                "privacy.env_disabled_notice",
                                &[("var", var)],
                            )),
                    )
            }))
            .when(status.dev_build, |row| {
                row.child(
                    div()
                        .debug_selector(|| "PRIVACY_DEV_NOTICE".into())
                        .flex()
                        .items_center()
                        .gap(rems(6. / 16.))
                        .text_size(theme::text_meta())
                        .text_color(theme::muted())
                        .child(
                            svg()
                                .flex_none()
                                .path("info.svg")
                                .size(rems(12. / 16.))
                                .text_color(theme::muted()),
                        )
                        .child(crate::i18n::text("privacy.dev_build_notice")),
                )
            })
            .into_any_element()
    }

    fn render_payload_row(&self, status: &TelemetryStatus, effective_enabled: bool) -> AnyElement {
        let caption = if let Some(var) = status.env_override.as_ref() {
            crate::i18n::tf(
                "privacy.payload_caption_env_disabled",
                &[("var", var.clone())],
            )
        } else if !effective_enabled {
            crate::i18n::text("privacy.payload_caption_disabled").to_owned()
        } else if status.dev_build {
            crate::i18n::tf(
                "privacy.payload_caption_dev",
                &[("endpoint", status.endpoint.clone())],
            )
        } else {
            crate::i18n::tf(
                "privacy.payload_caption",
                &[("endpoint", status.endpoint.clone())],
            )
        };

        let preview_lines: Vec<String> = status
            .payload_preview
            .lines()
            .map(ToOwned::to_owned)
            .collect();

        div()
            .flex()
            .flex_col()
            .gap(rems(8. / 16.))
            .px_4()
            .py_3()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .debug_selector(|| "PRIVACY_PAYLOAD_CAPTION".into())
                            .min_w(px(0.))
                            .whitespace_normal()
                            .text_size(theme::text_meta())
                            .text_color(theme::muted())
                            .child(caption),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .child(crate::i18n::text("privacy.no_ip_logged")),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "PRIVACY_PAYLOAD_PREVIEW".into())
                    .w_full()
                    .flex()
                    .flex_col()
                    .rounded(rems(8. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::bg())
                    .px(rems(12. / 16.))
                    .py(rems(10. / 16.))
                    .font_family(theme::UI_MONOSPACE_FONT)
                    .text_size(rems(11.5 / 16.))
                    .line_height(rems(17. / 16.))
                    .text_color(if effective_enabled {
                        theme::text()
                    } else {
                        theme::muted()
                    })
                    .when(!effective_enabled, |box_| box_.opacity(0.65))
                    .children(preview_lines.into_iter().map(|line| div().child(line))),
            )
            .into_any_element()
    }

    fn render_install_id_row(
        &self,
        status: &TelemetryStatus,
        loaded: bool,
        effective_enabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let pane = cx.entity();
        let reset_disabled =
            !loaded || !effective_enabled || status.install_id.is_none() || self.busy;
        let value_el = if effective_enabled {
            if let Some(install_id) = status.install_id.clone() {
                div()
                    .debug_selector(|| "PRIVACY_INSTALL_ID".into())
                    .font_family(theme::UI_MONOSPACE_FONT)
                    .text_size(theme::text_meta())
                    .text_color(theme::muted())
                    .child(install_id)
            } else {
                div()
                    .debug_selector(|| "PRIVACY_INSTALL_ID_CLEARED".into())
                    .text_size(theme::text_meta())
                    .text_color(theme::faint())
                    .child(crate::i18n::text("privacy.install_id_cleared"))
            }
        } else {
            let cleared_copy = if status.env_override.is_some() {
                crate::i18n::text("privacy.install_id_env_disabled")
            } else {
                crate::i18n::text("privacy.install_id_cleared")
            };
            div()
                .debug_selector(|| "PRIVACY_INSTALL_ID_CLEARED".into())
                .whitespace_normal()
                .text_size(theme::text_meta())
                .text_color(theme::faint())
                .child(cleared_copy)
        };

        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(rems(20. / 16.))
            .px_4()
            .py_3()
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(rems(3. / 16.))
                    .child(
                        div()
                            .text_size(theme::text_body())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::text())
                            .child(crate::i18n::text("privacy.install_id_label")),
                    )
                    .child(value_el),
            )
            .child(
                div().flex_none().child(
                    Button::new("privacy-reset-id", crate::i18n::text("privacy.reset_id"))
                        .debug_selector("PRIVACY_RESET_ID")
                        .size(ButtonSize::Compact)
                        .variant(ButtonVariant::Secondary)
                        .disabled(reset_disabled)
                        .on_press(move |_, cx| {
                            pane.update(cx, |pane, pane_cx| pane.reset_id(pane_cx));
                        }),
                ),
            )
            .into_any_element()
    }
}

fn stays_local_row(
    icon: &'static str,
    title_key: &'static str,
    body_key: &'static str,
) -> AnyElement {
    div()
        .flex()
        .items_start()
        .gap(rems(12. / 16.))
        .px_4()
        .py_3()
        .child(
            div().flex_none().pt(rems(2. / 16.)).child(
                svg()
                    .flex_none()
                    .path(icon)
                    .size(rems(14. / 16.))
                    .text_color(theme::accent()),
            ),
        )
        .child(
            div()
                .min_w(px(0.))
                .flex_1()
                .flex()
                .flex_col()
                .gap(rems(2. / 16.))
                .child(
                    div()
                        .text_size(theme::text_body())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::text())
                        .child(crate::i18n::text(title_key)),
                )
                .child(
                    div()
                        .whitespace_normal()
                        .text_size(theme::text_meta())
                        .line_height(rems(17. / 16.))
                        .text_color(theme::muted())
                        .child(crate::i18n::text(body_key)),
                ),
        )
        .into_any_element()
}

impl Render for PrivacyPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let loaded = self.app_store.read(cx).telemetry.is_some();
        let status = self.status(cx);
        let effective_enabled = status.enabled && status.env_override.is_none();

        div()
            .debug_selector(|| "PRIVACY_PANE".into())
            .flex()
            .flex_col()
            .gap_5()
            .child(PaneHeader::new(
                crate::i18n::text("privacy.title"),
                crate::i18n::text("privacy.subtitle"),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(settings_section_heading(crate::i18n::text(
                        "privacy.telemetry_section",
                    )))
                    .child(SettingsCard::new([
                        self.render_heartbeat_row(&status, loaded, effective_enabled, cx),
                        self.render_payload_row(&status, effective_enabled),
                        self.render_install_id_row(&status, loaded, effective_enabled, cx),
                    ])),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(settings_section_heading(crate::i18n::text(
                        "privacy.local_section",
                    )))
                    .child(SettingsCard::new([
                        stays_local_row(
                            "lock.svg",
                            "privacy.local_only_title",
                            "privacy.local_only_body",
                        ),
                        stays_local_row(
                            "shield-check.svg",
                            "privacy.no_tracking_title",
                            "privacy.no_tracking_body",
                        ),
                    ])),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surfaces::settings::page::SettingsPane;
    use crate::theme::ThemeIntent;
    use crate::NativeRoot;
    use gpui::{px, size, Modifiers, TestAppContext, VisualTestContext};

    fn wait_until(
        visual: &mut VisualTestContext,
        mut predicate: impl FnMut(&mut VisualTestContext) -> bool,
    ) {
        for _ in 0..200 {
            visual.run_until_parked();
            if predicate(visual) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("condition not satisfied before timeout");
    }

    fn is_lowercase_uuid_v4(value: &str) -> bool {
        let bytes = value.as_bytes();
        bytes.len() == 36
            && bytes[8] == b'-'
            && bytes[13] == b'-'
            && bytes[18] == b'-'
            && bytes[23] == b'-'
            && bytes[14] == b'4'
            && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
            && bytes.iter().enumerate().all(|(idx, b)| {
                matches!(idx, 8 | 13 | 18 | 23) || matches!(b, b'0'..=b'9' | b'a'..=b'f')
            })
    }

    fn click_selector(visual: &mut VisualTestContext, selector: &'static str) {
        let bounds = visual.debug_bounds(selector).expect(selector);
        visual.simulate_click(bounds.center(), Modifiers::default());
        visual.run_until_parked();
    }

    #[test]
    fn privacy_guide_url_follows_active_locale() {
        runner_app::i18n::testing::with_locale(runner_app::i18n::Locale::English, || {
            assert_eq!(privacy_guide_url(), PRIVACY_GUIDE_URL_EN);
        });
        runner_app::i18n::testing::with_locale(runner_app::i18n::Locale::SimplifiedChinese, || {
            assert_eq!(privacy_guide_url(), PRIVACY_GUIDE_URL_ZH);
        });
    }

    fn is_privacy_idle(
        handle: &gpui::WindowHandle<NativeRoot>,
        visual: &VisualTestContext,
    ) -> bool {
        handle
            .read_with(visual, |root, cx| {
                root.settings_page
                    .privacy
                    .as_ref()
                    .is_some_and(|pane| !pane.read(cx).busy)
            })
            .unwrap_or(false)
    }

    fn dnt_telemetry_service() -> std::sync::Arc<runner_daemon::telemetry::TelemetryService> {
        let mut vars = std::collections::BTreeMap::new();
        vars.insert("DO_NOT_TRACK".to_owned(), "1".to_owned());
        std::sync::Arc::new(runner_daemon::telemetry::TelemetryService::with_config(
            runner_daemon::telemetry::TelemetryConfig {
                dev_build: true,
                env: runner_daemon::telemetry::TelemetryEnvSource::Fixed(vars),
                network_enabled: false,
                ..runner_daemon::telemetry::TelemetryConfig::default()
            },
        ))
    }

    #[gpui::test]
    fn privacy_pane_toggle_reset_id_and_light_theme(cx: &mut TestAppContext) {
        let _theme_guard = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let store = crate::app_store::test_lifecycle_store(cx, temp.path());
        // Startup does not synchronously query telemetry on the GPUI thread.
        assert!(store.read_with(cx, |store, _| store.telemetry.is_none()));
        let handle = cx.add_window(|window, cx| {
            window.resize(size(px(1200.), px(960.)));
            let mut root = NativeRoot::new(
                "main".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.enter_settings_pane(SettingsPane::Privacy, window, cx);
            root
        });
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1200.), px(960.)));
        let _ = handle.update(&mut visual, |_, window, _| window.refresh());
        wait_until(&mut visual, |visual| {
            let _ = handle.update(visual, |_, window, _| window.refresh());
            is_privacy_idle(&handle, visual)
                && store.read_with(visual, |store, _| store.telemetry.is_some())
                && visual.debug_bounds("PRIVACY_INSTALL_ID").is_some()
        });

        assert!(visual.debug_bounds("PRIVACY_PANE").is_some());
        assert!(visual.debug_bounds("PRIVACY_HEARTBEAT_TOGGLE").is_some());
        assert!(visual.debug_bounds("PRIVACY_GUIDE_LINK").is_some());
        assert!(visual.debug_bounds("PRIVACY_DEV_NOTICE").is_some());
        assert!(visual.debug_bounds("PRIVACY_PAYLOAD_PREVIEW").is_some());
        assert!(visual.debug_bounds("PRIVACY_RESET_ID").is_some());

        let initial_id = store
            .read_with(&visual, |store, _| {
                store.telemetry.as_ref().and_then(|s| s.install_id.clone())
            })
            .expect("initial install id");
        assert!(is_lowercase_uuid_v4(&initial_id), "initial_id={initial_id}");

        // Reset ID rotates the UUID and updates the live payload preview.
        click_selector(&mut visual, "PRIVACY_RESET_ID");
        wait_until(&mut visual, |visual| {
            is_privacy_idle(&handle, visual)
                && store.read_with(visual, |store, _| {
                    store
                        .telemetry
                        .as_ref()
                        .and_then(|s| s.install_id.as_deref())
                        .is_some_and(|id| id != initial_id)
                })
        });
        let rotated_id = store.read_with(&visual, |store, _| {
            let status = store.telemetry.as_ref().unwrap();
            let id = status.install_id.clone().unwrap();
            assert!(status.payload_preview.contains(&id));
            id
        });
        assert_ne!(rotated_id, initial_id);

        // Toggling off clears Install ID in daemon SQLite and updates UI.
        click_selector(&mut visual, "PRIVACY_HEARTBEAT_TOGGLE");
        wait_until(&mut visual, |visual| {
            let _ = handle.update(visual, |_, window, _| window.refresh());
            is_privacy_idle(&handle, visual)
                && store.read_with(visual, |store, _| {
                    store
                        .telemetry
                        .as_ref()
                        .is_some_and(|s| !s.enabled && s.install_id.is_none())
                })
                && visual.debug_bounds("PRIVACY_INSTALL_ID_CLEARED").is_some()
        });
        assert!(visual.debug_bounds("PRIVACY_INSTALL_ID").is_none());

        // Toggling back on creates a fresh Install ID.
        click_selector(&mut visual, "PRIVACY_HEARTBEAT_TOGGLE");
        wait_until(&mut visual, |visual| {
            let _ = handle.update(visual, |_, window, _| window.refresh());
            is_privacy_idle(&handle, visual)
                && store.read_with(visual, |store, _| {
                    store
                        .telemetry
                        .as_ref()
                        .is_some_and(|s| s.enabled && s.install_id.is_some())
                })
                && visual.debug_bounds("PRIVACY_INSTALL_ID").is_some()
        });
        let regenerated_id = store
            .read_with(&visual, |store, _| {
                store.telemetry.as_ref().and_then(|s| s.install_id.clone())
            })
            .expect("regenerated install id");
        assert_ne!(regenerated_id, rotated_id);

        // Runner Light renders cleanly without layout regressions.
        store.update(&mut visual, |store, cx| {
            store.update_settings(
                |settings| {
                    settings.app_theme = ThemeIntent::Light;
                    true
                },
                false,
                cx,
            );
        });
        let _ = handle.update(&mut visual, |_, window, _| window.refresh());
        visual.run_until_parked();
        assert!(visual.debug_bounds("PRIVACY_PANE").is_some());
        assert!(visual.debug_bounds("PRIVACY_PAYLOAD_PREVIEW").is_some());
    }

    #[gpui::test]
    fn privacy_pane_env_override_locks_toggle(cx: &mut TestAppContext) {
        let _theme_guard = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let store = crate::app_store::test_lifecycle_store_with_telemetry(
            cx,
            temp.path(),
            Some(dnt_telemetry_service()),
        );
        let handle = cx.add_window(|window, cx| {
            window.resize(size(px(1200.), px(960.)));
            let mut root = NativeRoot::new(
                "main".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.enter_settings_pane(SettingsPane::Privacy, window, cx);
            root
        });
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1200.), px(960.)));
        let _ = handle.update(&mut visual, |_, window, _| window.refresh());
        wait_until(&mut visual, |visual| {
            let _ = handle.update(visual, |_, window, _| window.refresh());
            is_privacy_idle(&handle, visual)
                && store.read_with(visual, |store, _| store.telemetry.is_some())
                && visual.debug_bounds("PRIVACY_ENV_NOTICE").is_some()
                && visual.debug_bounds("PRIVACY_INSTALL_ID_CLEARED").is_some()
        });

        // Clicking the disabled toggle must not turn telemetry back on.
        click_selector(&mut visual, "PRIVACY_HEARTBEAT_TOGGLE");
        let status = store
            .read_with(&visual, |store, _| store.client.telemetry_status())
            .unwrap();
        assert!(!status.enabled);
        assert_eq!(status.env_override.as_deref(), Some("DO_NOT_TRACK=1"));
        assert!(status.install_id.is_none());
    }

    #[gpui::test]
    fn privacy_pane_reentry_during_pending_mutation_does_not_strand_busy(cx: &mut TestAppContext) {
        let _theme_guard = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let store = crate::app_store::test_lifecycle_store(cx, temp.path());
        let handle = cx.add_window(|window, cx| {
            window.resize(size(px(1200.), px(960.)));
            let mut root = NativeRoot::new(
                "main".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.enter_settings_pane(SettingsPane::Privacy, window, cx);
            root
        });
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1200.), px(960.)));
        wait_until(&mut visual, |visual| {
            let _ = handle.update(visual, |_, window, _| window.refresh());
            is_privacy_idle(&handle, visual)
                && store.read_with(visual, |store, _| store.telemetry.is_some())
                && visual.debug_bounds("PRIVACY_INSTALL_ID").is_some()
        });

        let initial_id = store
            .read_with(&visual, |store, _| {
                store.telemetry.as_ref().and_then(|s| s.install_id.clone())
            })
            .expect("initial install id");

        // Start a Reset ID mutation and immediately leave & re-enter Privacy (calling refresh())
        // before the mutation completes.
        let _ = handle.update(&mut visual, |root, window, cx| {
            let privacy = root.settings_page.privacy.clone().unwrap();
            privacy.update(cx, |pane, pane_cx| pane.reset_id(pane_cx));
            assert!(privacy.read(cx).busy);
            root.enter_settings_pane(SettingsPane::About, window, cx);
            root.enter_settings_pane(SettingsPane::Privacy, window, cx);
        });

        // Busy must clear once the mutation completes, and the rotated ID must apply.
        wait_until(&mut visual, |visual| {
            let _ = handle.update(visual, |_, window, _| window.refresh());
            is_privacy_idle(&handle, visual)
                && store.read_with(visual, |store, _| {
                    store
                        .telemetry
                        .as_ref()
                        .and_then(|s| s.install_id.as_deref())
                        .is_some_and(|id| id != initial_id)
                })
        });

        // Controls must remain interactive after re-entry.
        click_selector(&mut visual, "PRIVACY_HEARTBEAT_TOGGLE");
        wait_until(&mut visual, |visual| {
            let _ = handle.update(visual, |_, window, _| window.refresh());
            is_privacy_idle(&handle, visual)
                && store.read_with(visual, |store, _| {
                    store
                        .telemetry
                        .as_ref()
                        .is_some_and(|s| !s.enabled && s.install_id.is_none())
                })
        });
    }
}
