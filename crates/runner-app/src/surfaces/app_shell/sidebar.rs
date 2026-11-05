use super::*;

impl NativeRoot {
    pub(super) fn render_app_sidebar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Option<AnyElement>, Option<f32>) {
        let visible = !self.front_page_sidebar_collapsed() || self.sidebar_preview_open;
        let visibility_target = if visible { 1. } else { 0. };
        let (visibility, animating) = self.sidebar_visibility.animate_to(
            visibility_target,
            Instant::now(),
            Duration::from_millis(SIDEBAR_TRANSITION_MS),
        );
        if animating {
            window.request_animation_frame();
        }
        let show_panel = visible || animating;
        let full_width = self.front_page_sidebar_width(cx) * self.settings(cx).app_zoom;
        let width = full_width * visibility;
        if !show_panel {
            return (
                Some(
                    div()
                        .id("app-sidebar")
                        .relative()
                        .w(px(width))
                        .h_full()
                        .flex_none()
                        .overflow_hidden()
                        .into_any_element(),
                ),
                None,
            );
        }
        let preview = self.front_page_sidebar_collapsed()
            && (self.sidebar_preview_open || self.sidebar_preview_peeking);
        let content = self.cached_region(
            "sidebar",
            gpui::StyleRefinement::default()
                .w(px(full_width))
                .h_full()
                .flex_none(),
            move |root, window, cx| root.render_sidebar_content(full_width, window, cx),
            cx,
        );
        let mut sidebar = div()
            .id("app-sidebar")
            .relative()
            .w(px(width))
            .h_full()
            .flex_none()
            .overflow_hidden()
            .opacity(visibility)
            .bg(if theme::is_glass() {
                gpui::transparent_black()
            } else {
                theme::sidebar()
            })
            .map(|element| {
                #[cfg(test)]
                let element = crate::theme_snapshot::record_fill("APP_SIDEBAR", element);
                element
            })
            .child(content);
        let divider = visible.then_some(width);
        if preview {
            sidebar = sidebar
                .bg(theme::chrome())
                .absolute()
                .left_0()
                .top_0()
                .rounded_tr(px(12. * self.settings(cx).app_zoom))
                .rounded_br(px(12. * self.settings(cx).app_zoom))
                .shadow_2xl()
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    if !*hovered && this.front_page_sidebar_collapsed() {
                        this.sidebar_preview_open = false;
                        cx.notify();
                    }
                }));
            return (
                Some(deferred(sidebar).with_priority(1).into_any_element()),
                divider,
            );
        }
        (Some(sidebar.into_any_element()), divider)
    }

    pub(super) fn render_sidebar_brand(
        &self,
        update_version: Option<String>,
        cx: &Context<Self>,
    ) -> Div {
        let zoom = self.settings(cx).app_zoom;
        let updater = global_updater(cx);
        let update_hint = update_version
            .filter(|_| self.remote_client.is_local())
            .map(|version| {
                let click_updater = updater.clone();
                let click_root = cx.weak_entity();
                #[cfg(target_os = "macos")]
                let tooltip = crate::platform_ui::update_hint_tooltip(&version);
                #[cfg(any(windows, target_os = "linux"))]
                let tooltip =
                    crate::platform_ui::update_hint_tooltip(&version, updater.read(cx).state());
                Tooltip::new(
                    "sidebar-update-tooltip",
                    tooltip,
                    div()
                        .id("sidebar-update")
                        .when(cfg!(test), |button| {
                            button.debug_selector(|| "SIDEBAR_UPDATE".into())
                        })
                        .flex_none()
                        .size(rems(24. / 16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_sm()
                        .border_1()
                        .border_color(gpui::transparent_black())
                        .cursor_pointer()
                        .hover(|button| {
                            button
                                .border_color(alpha(theme::accent(), 0.4))
                                .bg(alpha(theme::accent(), 0.1))
                        })
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |_, _window, cx| {
                            cx.stop_propagation();
                            if !click_root
                                .upgrade()
                                .is_some_and(|root| root.read(cx).remote_client.is_local())
                            {
                                return;
                            }
                            #[cfg(target_os = "macos")]
                            crate::platform_ui::activate_update_hint(&click_updater, cx);
                            #[cfg(any(windows, target_os = "linux"))]
                            crate::platform_ui::activate_update_hint(&click_updater, _window, cx);
                        })
                        .child(
                            svg()
                                .path("circle-arrow-down.svg")
                                .flex_none()
                                .size(px(14. * zoom))
                                .text_color(theme::accent()),
                        ),
                )
            });
        let active_only = self.front_page_sidebar().read(cx).active_only(cx);
        let active_toggle = IconButton::new("sidebar-active-toggle", "list-filter.svg")
            .size(IconButtonSize::Md)
            .variant(if active_only {
                runner_app::ui::ButtonVariant::Secondary
            } else {
                runner_app::ui::ButtonVariant::Ghost
            })
            .tooltip(crate::i18n::text(if active_only {
                "ui.show_all_items"
            } else {
                "ui.show_active_only"
            }))
            .on_press({
                let owner = cx.entity();
                move |_, cx| {
                    owner.update(cx, |this, cx| {
                        this.clear_sidebar_drag("change-view", cx);
                        if this.remote_client.is_active() {
                            this.front_page_sidebar()
                                .update(cx, |sidebar, cx| sidebar.toggle_active_only(cx));
                            this.sync_sidebar_shortcut_rows(cx);
                            cx.notify();
                            return;
                        }
                        this.app_store.update(cx, |store, cx| {
                            store.update_settings(
                                |settings| {
                                    settings.sidebar_active_only = !settings.sidebar_active_only;
                                    true
                                },
                                true,
                                cx,
                            );
                        });
                        this.sync_sidebar_shortcut_rows(cx);
                        cx.notify();
                    });
                }
            });
        let search_button = div()
            .id("sidebar-search")
            .debug_selector(|| "SIDEBAR_SEARCH".into())
            .group("sidebar-search")
            .tab_index(0)
            // A click leaves keyboard focus where it was, so dismissing the palette returns
            // there instead of to this button.
            .capture_any_mouse_down(|_, window, _| window.prevent_default())
            .flex_none()
            .size(px(24. * self.settings(cx).app_zoom))
            .flex()
            .items_center()
            .justify_center()
            .rounded_sm()
            .border_1()
            .border_color(gpui::transparent_black())
            .cursor_pointer()
            .text_color(theme::muted())
            .hover(|button| {
                button
                    .border_color(theme::chrome_selected_border())
                    .bg(theme::chrome_hover())
                    .text_color(theme::text())
            })
            .focus_visible(|button| {
                button
                    .border_color(theme::chrome_selected_border())
                    .bg(theme::chrome_hover())
                    .text_color(theme::text())
            })
            .on_click(cx.listener(|this, _, window, cx| {
                this.show_command_palette(window, cx);
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    this.show_command_palette(window, cx);
                }
            }))
            .child(
                svg()
                    .path("search.svg")
                    .size(px(14. * self.settings(cx).app_zoom))
                    .text_color(theme::muted())
                    .group_hover("sidebar-search", |icon| icon.text_color(theme::text())),
            );
        div()
            .when(cfg!(test), |brand| {
                brand.debug_selector(|| "SIDEBAR_HEADER".into())
            })
            .flex_none()
            .px_5()
            .pb_5()
            .pt_1()
            .flex()
            .items_center()
            .gap_2()
            .child(
                svg()
                    .when(cfg!(test), |mark| {
                        mark.debug_selector(|| "SIDEBAR_BRAND_MARK".into())
                    })
                    .flex_none()
                    .path("brand-mark.svg")
                    .size(px(20. * self.settings(cx).app_zoom))
                    .text_color(theme::accent()),
            )
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .when(cfg!(test), |name| {
                                name.debug_selector(|| "SIDEBAR_NAME".into())
                            })
                            .min_w(px(0.))
                            .truncate()
                            .text_size(theme::text_heading())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child("Runner"),
                    )
                    .children(update_hint),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(rems(4. / 16.))
                    .child(active_toggle)
                    .child(Tooltip::new(
                        "sidebar-search-tooltip",
                        keymap::effective_binding(
                            "command-palette",
                            &self.settings(cx).keymap_overrides,
                        )
                        .map_or_else(
                            || crate::i18n::text("ui.search").to_owned(),
                            |combo| {
                                crate::i18n::tf(
                                    "format.search",
                                    &[("shortcut", (keymap::format_combo(&combo)).to_string())],
                                )
                            },
                        ),
                        search_button,
                    )),
            )
    }

    pub(super) fn render_sidebar_content(
        &mut self,
        full_width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.sync_sidebar_shortcut_rows(cx);
        let titlebar = self.render_sidebar_titlebar(window, cx);
        let zoom = self.settings(cx).app_zoom;
        let updater = global_updater(cx);
        let update_version =
            app_update_hint_version(updater.read(cx).available()).map(str::to_owned);
        let brand = self.render_sidebar_brand(update_version, cx);
        let enabled = self.settings(cx).model_runtimes();
        let visible_usage_runtimes = visible_usage_runtimes(&self.usage_installed, &enabled);
        let show_usage = !visible_usage_runtimes.is_empty() && !self.front_page_sidebar_collapsed();
        let anchor_owner = cx.entity();
        let usage = show_usage.then(|| {
            let snapshot = self.app_store.read(cx).usage.clone();
            let trigger = sidebar_usage_element(&snapshot, &visible_usage_runtimes, zoom)
                .occlude()
                .cursor_pointer()
                .hover(|button| button.bg(theme::chrome_hover()))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.usage_open = !this.usage_open;
                    if this.usage_open {
                        let core = this.core(cx).clone();
                        let _ = core.usage_refresh(RefreshReason::Open);
                        let _ = this.core(cx).runtime_check_updates(false);
                    }
                    cx.notify();
                }));
            div()
                .relative()
                .w(px((visible_usage_runtimes.len() as f32 * 52. + 20.)
                    .min(self.front_page_sidebar_width(cx) - 92.)
                    * zoom))
                .flex_none()
                .child(trigger)
                .child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, _, cx| {
                            anchor_owner.update(cx, |this, _| this.usage_anchor = Some(bounds));
                        },
                    )
                    .absolute()
                    .inset_0(),
                )
        });
        let footer = crate::platform_ui::sidebar_section()
            .when(cfg!(test), |footer| {
                footer.debug_selector(|| "SIDEBAR_FOOTER".into())
            })
            .px_3()
            .child(
                div()
                    .w_full()
                    .h(px(1.))
                    .flex()
                    .children([90., 270.].map(|angle| {
                        div().flex_1().h_full().bg(linear_gradient(
                            angle,
                            linear_color_stop(alpha(theme::chrome_selected_border(), 0.), 0.),
                            linear_color_stop(theme::chrome_selected_border(), 1.),
                        ))
                    })),
            )
            .child(
                div()
                    .when(cfg!(test), |row| {
                        row.debug_selector(|| "SIDEBAR_FOOTER_ROW".into())
                    })
                    .w_full()
                    .pt_2()
                    .flex()
                    .items_center()
                    .relative()
                    .children(usage)
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(4. * zoom))
                            .child(self.remote_client.render_host_button(cx))
                            .child(self.render_settings_button(cx)),
                    )
                    .child({
                        let owner = cx.entity();
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, _, cx| {
                                owner.update(cx, |this, _| {
                                    this.remote_client.ui.footer_left = Some(bounds.left())
                                });
                            },
                        )
                        .absolute()
                        .inset_0()
                    }),
            );
        // The content keeps its full width while the wrapper animates, so the
        // transition clips instead of squashing every row; a squashed row
        // shrinks its icons to zero and gpui refuses to paint them (#512).
        let content = div()
            .w(px(full_width))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .pb_3()
            .children(titlebar)
            .child(brand)
            .child(self.front_page_sidebar())
            .child(footer)
            .children(
                (self.usage_open && show_usage)
                    .then(|| {
                        self.usage_anchor.map(|anchor| {
                            let owner = cx.entity();
                            popup_layer(
                                anchor,
                                window,
                                px(340. * zoom),
                                self.render_usage_popover(window, cx),
                                Rc::new(move |_, cx| {
                                    owner.update(cx, |this, cx| {
                                        this.usage_open = false;
                                        cx.notify();
                                    });
                                }),
                            )
                        })
                    })
                    .flatten(),
            );
        content.into_any_element()
    }

    pub(super) fn finish_sidebar_resize(&mut self, cx: &mut Context<Self>) {
        if !self.sidebar_resizing {
            return;
        }
        self.sidebar_resizing = false;
        cx.notify();
    }

    /// How far right of the sidebar's edge the border it resizes sits: the
    /// island area's left edge, past the gap, when there are islands.
    pub(crate) fn sidebar_border_offset(&self, cx: &App) -> f32 {
        if self.front_page_has_islands() && !self.front_page_sidebar_collapsed() {
            runner_app::ui::work_card::SIDEBAR_GAP * self.settings(cx).app_zoom
        } else {
            0.
        }
    }

    pub(crate) fn render_sidebar_resize_handle(&self, divider: f32, cx: &App) -> AnyElement {
        let zoom = self.settings(cx).app_zoom;
        let left = divider + self.sidebar_border_offset(cx) - resize_strip_inset(zoom);
        resize_strip(
            "sidebar-resize",
            ResizeAxis::Columns,
            self.sidebar_resizing,
            zoom,
            None,
            false,
        )
        .id("sidebar-resize")
        .when(cfg!(test), |handle| {
            handle.debug_selector(|| "SIDEBAR_RESIZE_GUTTER".into())
        })
        .map(|handle| {
            #[cfg(any(windows, target_os = "linux"))]
            let handle = handle.occlude();
            handle
        })
        .absolute()
        .top_0()
        .left(px(left))
        .on_drag(
            SidebarResizeDrag,
            |drag: &SidebarResizeDrag, _, _, cx: &mut App| cx.new(|_| drag.clone()),
        )
        .into_any_element()
    }

    pub(super) fn render_sidebar_preview_trigger(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        (self.front_page_sidebar_collapsed()
            && (self.route != AppRoute::Settings || self.remote_client.is_active()))
        .then(|| {
            div()
                .id("sidebar-preview-trigger")
                .absolute()
                .left_0()
                .top_0()
                .w(px(16. * self.settings(cx).app_zoom))
                .h_full()
                .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                    this.sidebar_preview_trigger_hovered = *hovered;
                    if !*hovered {
                        this.cancel_sidebar_preview_dwell();
                    }
                }))
                .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                    // A drag that reaches the edge, such as a text selection,
                    // must not open the preview.
                    if event.pressed_button.is_some() {
                        this.cancel_sidebar_preview_dwell();
                    } else {
                        this.arm_sidebar_preview_dwell(cx);
                    }
                }))
                .into_any_element()
        })
    }

    pub(super) fn cancel_sidebar_preview_dwell(&mut self) {
        self.sidebar_preview_dwell = 0;
    }

    pub(super) fn arm_sidebar_preview_dwell(&mut self, cx: &mut Context<Self>) {
        if self.sidebar_preview_dwell != 0 || self.sidebar_preview_open {
            return;
        }
        let generation = self.sidebar_preview_dwell_generation.wrapping_add(1).max(1);
        self.sidebar_preview_dwell_generation = generation;
        self.sidebar_preview_dwell = generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(SIDEBAR_PREVIEW_DWELL_MS))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.sidebar_preview_dwell != generation {
                    return;
                }
                this.sidebar_preview_dwell = 0;
                if this.sidebar_preview_trigger_hovered && this.front_page_sidebar_collapsed() {
                    this.sidebar_preview_open = true;
                    this.sidebar_preview_peeking = true;
                    cx.notify();
                }
            });
        })
        .detach();
    }
}
