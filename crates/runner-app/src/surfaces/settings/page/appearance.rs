use super::*;

pub(super) fn missions_settings_pane(default_crew: Entity<StyledSelect>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_5()
        .debug_selector(|| "SETTINGS_MISSIONS_PANE".into())
        .child(PaneHeader::new(crate::i18n::text("ui.missions"), ""))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rems(14. / 16.))
                .child(
                    div()
                        .debug_selector(|| "SETTINGS_MISSIONS_DEFAULTS_HEADING".into())
                        .text_size(theme::text_heading())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text())
                        .child(crate::i18n::text("ui.defaults")),
                )
                .child(
                    div()
                        .debug_selector(|| "SETTINGS_MISSIONS_CARD".into())
                        .child(SettingsCard::new(vec![SettingsRow::new(
                            crate::i18n::text("ui.default_crew"),
                            div()
                                .debug_selector(|| "SETTINGS_DEFAULT_CREW".into())
                                .child(default_crew),
                        )
                        .subtitle(crate::i18n::text(
                            "ui.pre_selected_when_starting_a_new_mission",
                        ))
                        .into_any_element()])),
                ),
        )
        .into_any_element()
}

impl NativeRoot {
    pub(super) fn render_missions_settings(&self, _cx: &mut Context<Self>) -> AnyElement {
        missions_settings_pane(self.settings_page.default_crew.clone())
    }

    pub(super) fn render_appearance_settings(
        &self,
        gallery_columns: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (light_app_theme, dark_app_theme) = {
            let settings = self.settings(cx);
            (settings.light_app_theme, settings.dark_app_theme)
        };
        let active_is_light = theme::active_variant().is_light();
        let root = cx.entity();
        let light_cards: Vec<_> = theme::LightTheme::ALL
            .into_iter()
            .map(|choice| {
                let root = root.clone();
                ThemeGalleryCard {
                    variant: choice.variant(),
                    key: choice.key(),
                    label: choice.label(),
                    selected: choice == light_app_theme,
                    mode_tag: "LIGHT",
                    on_select: Rc::new(move |cx| {
                        root.update(cx, |this, cx| this.set_light_theme(choice, cx));
                    }),
                }
            })
            .collect();
        let dark_cards: Vec<_> = theme::DarkTheme::ALL
            .into_iter()
            .map(|choice| {
                let root = root.clone();
                ThemeGalleryCard {
                    variant: choice.variant(),
                    key: choice.key(),
                    label: choice.label(),
                    selected: choice == dark_app_theme,
                    mode_tag: "DARK",
                    on_select: Rc::new(move |cx| {
                        root.update(cx, |this, cx| this.set_dark_theme(choice, cx));
                    }),
                }
            })
            .collect();

        div()
            .debug_selector(|| "SETTINGS_APPEARANCE_PANE".into())
            .flex()
            .flex_col()
            .gap_6()
            .child(PaneHeader::new(
                crate::i18n::text("ui.appearance"),
                crate::i18n::text(
                    "ui.pick_a_theme_for_each_mode_a_theme_sets_the_app_and_terminal_colours_together",
                ),
            ))
            .child(SettingsCard::new({
                let rows = vec![SettingsRow::new(crate::i18n::text("ui.theme"), self.render_theme_segmented(cx))
                    .subtitle(crate::i18n::text("ui.match_the_os_or_pin_to_light_or_dark"))
                    .into_any_element()];
                #[cfg(target_os = "macos")]
                let rows = {
                    let mut rows = rows;
                    rows.push(SettingsRow::new(crate::i18n::text("ui.window_material"), self.render_material_segmented(cx))
                        .subtitle(crate::i18n::text("ui.glass_adds_depth_to_the_sidebar_and_window_chrome"))
                        .into_any_element());
                    rows
                };
                rows
            }))
            .child(theme_gallery_section(
                crate::i18n::text("ui.light_theme"),
                "ui.used_when_the_app_is_light",
                "SETTINGS_LIGHT_THEME_GALLERY",
                "SETTINGS_LIGHT_THEME_META",
                active_is_light,
                gallery_columns,
                light_cards,
            ))
            .child(theme_gallery_section(
                crate::i18n::text("ui.dark_theme"),
                "ui.used_when_the_app_is_dark",
                "SETTINGS_DARK_THEME_GALLERY",
                "SETTINGS_DARK_THEME_META",
                !active_is_light,
                gallery_columns,
                dark_cards,
            ))
            .when(cfg!(target_os = "macos"), |page| page.child(
                div().text_size(theme::text_ui()).text_color(theme::muted())
                    .child(crate::i18n::text("ui.runner_follows_macos_reduce_transparency_when_enabled_glass_uses_the_solid_appearance")),
            ))
            .into_any_element()
    }

    #[cfg(target_os = "macos")]
    pub(super) fn render_material_segmented(&self, cx: &mut Context<Self>) -> AnyElement {
        use runner_app::appearance::WindowMaterial;
        let selected = self.settings(cx).window_material;
        div()
            .debug_selector(|| "SETTINGS_WINDOW_MATERIAL".into())
            .flex()
            .gap(rems(2. / 16.))
            .p(rems(2. / 16.))
            .rounded(rems(6. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg())
            .children(
                [
                    (WindowMaterial::Glass, crate::i18n::text("ui.glass")),
                    (WindowMaterial::Solid, crate::i18n::text("ui.solid")),
                ]
                .into_iter()
                .map(|(material, label)| {
                    let root = cx.entity();
                    let key_root = root.clone();
                    div()
                        .id(SharedString::from(format!("settings-material-{label}")))
                        .tab_index(0)
                        .px(rems(12. / 16.))
                        .py(rems(5. / 16.))
                        .rounded(rems(4. / 16.))
                        .text_size(theme::text_ui())
                        .when(material == selected, |item| item.bg(theme::raised()))
                        .cursor_pointer()
                        .child(label)
                        .on_click(move |event, _, cx| {
                            if !event.is_keyboard() {
                                root.update(cx, |this, cx| this.set_window_material(material, cx));
                            }
                        })
                        .on_key_down(move |event: &KeyDownEvent, _, cx| {
                            if is_plain_activation_key(event) {
                                cx.stop_propagation();
                                key_root
                                    .update(cx, |this, cx| this.set_window_material(material, cx));
                            }
                        })
                }),
            )
            .into_any_element()
    }

    #[cfg(target_os = "macos")]
    pub(super) fn set_window_material(
        &self,
        material: runner_app::appearance::WindowMaterial,
        cx: &mut Context<Self>,
    ) {
        self.update_app_settings(cx, true, |settings| {
            if settings.window_material == material {
                return false;
            }
            settings.window_material = material;
            true
        });
        runner_app::appearance::configure(material, cx);
        cx.refresh_windows();
    }

    pub(super) fn render_theme_segmented(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap(rems(2. / 16.))
            .rounded(rems(6. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg())
            .p(rems(2. / 16.))
            .children(
                [
                    (
                        ThemeIntent::Auto,
                        crate::i18n::text("ui.auto"),
                        "monitor.svg",
                    ),
                    (ThemeIntent::Light, crate::i18n::text("ui.light"), "sun.svg"),
                    (ThemeIntent::Dark, crate::i18n::text("ui.dark"), "moon.svg"),
                ]
                .into_iter()
                .map(|(intent, label, icon)| {
                    let active = intent == self.settings(cx).app_theme;
                    let foreground = if active {
                        theme::text()
                    } else {
                        theme::muted()
                    };
                    let hover_group = SharedString::from(format!(
                        "settings-theme-{}",
                        label.to_ascii_lowercase()
                    ));
                    let root = cx.entity();
                    let key_root = root.clone();
                    div()
                        .id(SharedString::from(format!("settings-theme-{label}")))
                        .group(hover_group.clone())
                        .tab_index(0)
                        .flex()
                        .items_center()
                        .gap(rems(6. / 16.))
                        .rounded(rems(4. / 16.))
                        .px(rems(10. / 16.))
                        .py(rems(5. / 16.))
                        .bg(if active {
                            theme::raised()
                        } else {
                            gpui::transparent_black()
                        })
                        .text_size(theme::text_ui())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(foreground)
                        .cursor(CursorStyle::PointingHand)
                        .hover(|button| button.text_color(theme::text()))
                        .child(
                            svg()
                                .path(icon)
                                .size(rems(12. / 16.))
                                .flex_none()
                                .text_color(foreground)
                                .group_hover(hover_group, |icon| icon.text_color(theme::text())),
                        )
                        .child(label)
                        .on_click(move |event, _, cx| {
                            if !event.is_keyboard() {
                                root.update(cx, |this, root_cx| {
                                    this.set_theme_intent(intent, root_cx)
                                });
                            }
                        })
                        .on_key_down(move |event: &KeyDownEvent, _, cx| {
                            if is_plain_activation_key(event) {
                                cx.stop_propagation();
                                key_root.update(cx, |this, root_cx| {
                                    this.set_theme_intent(intent, root_cx)
                                });
                            }
                        })
                }),
            )
            .into_any_element()
    }

    pub(super) fn render_terminal_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.settings(cx);
        let size = settings
            .terminal_font_size
            .clamp(TERMINAL_FONT_SIZE_MIN, TERMINAL_FONT_SIZE_MAX);
        let line_height = normalize_terminal_line_height(settings.terminal_line_height);
        let cursor_blink = settings.terminal_cursor_blink;
        let copy_on_select = settings.terminal_copy_on_select;
        let root = cx.entity();
        let size_stepper = {
            let (down, up) = (root.clone(), root.clone());
            Stepper::new(
                "settings-terminal-font-size",
                64.,
                stepper_value(size.to_string(), Some("px")),
                Rc::new(move |_, cx| {
                    down.update(cx, |this, cx| {
                        this.set_terminal_font_size(size.saturating_sub(1), cx)
                    });
                }),
                Rc::new(move |_, cx| {
                    up.update(cx, |this, cx| {
                        this.set_terminal_font_size(size.saturating_add(1), cx)
                    });
                }),
            )
            .decrement_disabled(size == TERMINAL_FONT_SIZE_MIN)
            .increment_disabled(size == TERMINAL_FONT_SIZE_MAX)
        };
        let line_height_stepper = {
            let (down, up) = (root.clone(), root.clone());
            Stepper::new(
                "settings-terminal-line-height",
                64.,
                stepper_value(format!("{line_height:.1}"), None),
                Rc::new(move |_, cx| {
                    down.update(cx, |this, cx| {
                        this.set_terminal_line_height(line_height - 0.1, cx)
                    });
                }),
                Rc::new(move |_, cx| {
                    up.update(cx, |this, cx| {
                        this.set_terminal_line_height(line_height + 0.1, cx)
                    });
                }),
            )
            .decrement_disabled(line_height <= TERMINAL_LINE_HEIGHT_MIN)
            .increment_disabled(line_height >= TERMINAL_LINE_HEIGHT_MAX)
        };
        let toggle = |id: &'static str, on: bool, set: fn(&mut Self, bool, &mut Context<Self>)| {
            let root = root.clone();
            Toggle::new(id, on)
                .on_change(move |enabled, _, cx| {
                    root.update(cx, |this, cx| set(this, enabled, cx));
                })
                .into_any_element()
        };

        let text = settings_section(
            crate::i18n::text("terminal.section_text"),
            "SETTINGS_TERMINAL_TEXT",
            false,
            vec![
                SettingsRow::new(
                    crate::i18n::text("ui.font_family"),
                    div()
                        .debug_selector(|| "SETTINGS_TERMINAL_FONT".into())
                        .child(self.settings_page.terminal_font.clone()),
                )
                .subtitle(crate::i18n::text(
                    "ui.typeface_used_by_the_embedded_terminal",
                ))
                .into_any_element(),
                SettingsRow::new(
                    crate::i18n::text("ui.font_size"),
                    ranged_stepper(
                        format!("{TERMINAL_FONT_SIZE_MIN}–{TERMINAL_FONT_SIZE_MAX}"),
                        size_stepper,
                    ),
                )
                .subtitle(crate::i18n::text("ui.glyph_size_for_the_embedded_terminal"))
                .into_any_element(),
                SettingsRow::new(
                    crate::i18n::text("terminal.line_height"),
                    ranged_stepper(
                        format!("{TERMINAL_LINE_HEIGHT_MIN:.1}–{TERMINAL_LINE_HEIGHT_MAX:.1}"),
                        line_height_stepper,
                    ),
                )
                .subtitle(crate::i18n::text("terminal.line_height_description"))
                .into_any_element(),
            ],
        );
        let cursor = settings_section(
            crate::i18n::text("terminal.section_cursor"),
            "SETTINGS_TERMINAL_CURSOR",
            false,
            vec![
                SettingsRow::new(
                    crate::i18n::text("ui.cursor_style"),
                    self.settings_page.terminal_cursor.clone(),
                )
                .subtitle(crate::i18n::text(
                    "ui.block_underline_or_bar_affects_the_prompt_caret_only",
                ))
                .into_any_element(),
                SettingsRow::new(
                    crate::i18n::text("terminal.blink_cursor"),
                    toggle(
                        "settings-terminal-cursor-blink",
                        cursor_blink,
                        Self::set_terminal_cursor_blink,
                    ),
                )
                .subtitle(crate::i18n::text("terminal.blink_cursor_description"))
                .into_any_element(),
            ],
        );
        #[cfg(target_os = "macos")]
        let keyboard = Some(settings_section(
            crate::i18n::text("terminal.section_keyboard"),
            "SETTINGS_TERMINAL_KEYBOARD",
            true,
            vec![SettingsRow::new(
                crate::i18n::text("terminal.option_as_meta"),
                div()
                    .debug_selector(|| "SETTINGS_OPTION_AS_META".into())
                    .child(self.settings_page.option_as_meta.clone()),
            )
            .subtitle(crate::i18n::text("terminal.option_as_meta_description"))
            .into_any_element()],
        ));
        #[cfg(not(target_os = "macos"))]
        let keyboard: Option<AnyElement> = None;
        let scrollback = settings_section(
            crate::i18n::text("terminal.section_scrollback_and_selection"),
            "SETTINGS_TERMINAL_SCROLLBACK",
            false,
            vec![
                SettingsRow::new(
                    crate::i18n::text("terminal.scrollback"),
                    self.settings_page.terminal_scrollback.clone(),
                )
                .subtitle(crate::i18n::text("terminal.scrollback_description"))
                .into_any_element(),
                SettingsRow::new(
                    crate::i18n::text("terminal.copy_on_select"),
                    toggle(
                        "settings-terminal-copy-on-select",
                        copy_on_select,
                        Self::set_terminal_copy_on_select,
                    ),
                )
                .subtitle(crate::i18n::text("terminal.copy_on_select_description"))
                .into_any_element(),
            ],
        );

        div()
            .flex()
            .flex_col()
            .gap_6()
            .debug_selector(|| "SETTINGS_TERMINAL_PANE".into())
            .child(PaneHeader::new(
                crate::i18n::text("ui.terminal"),
                crate::i18n::text(
                    "ui.text_cursor_keyboard_and_scrollback_for_the_embedded_terminal_terminal_colours_c",
                ),
            ))
            .child(text)
            .child(cursor)
            .children(keyboard)
            .child(scrollback)
            .into_any_element()
    }
}

struct ThemeGalleryCard {
    variant: theme::ThemeVariant,
    key: &'static str,
    label: &'static str,
    selected: bool,
    mode_tag: &'static str,
    on_select: Rc<dyn Fn(&mut gpui::App)>,
}

fn theme_gallery_section(
    heading: &'static str,
    meta_key: &'static str,
    selector: &'static str,
    meta_selector: &'static str,
    in_use: bool,
    columns: usize,
    cards: impl IntoIterator<Item = ThemeGalleryCard>,
) -> AnyElement {
    let base_meta = crate::i18n::text(meta_key);
    let meta: SharedString = if in_use {
        crate::i18n::tf("format.theme_in_use", &[("meta", base_meta.to_string())]).into()
    } else {
        base_meta.into()
    };
    let mut rows = Vec::new();
    let mut iter = cards.into_iter().map(theme_gallery_card);
    loop {
        let row: Vec<_> = iter.by_ref().take(columns.max(1)).collect();
        if row.is_empty() {
            break;
        }
        rows.push(
            div()
                .w_full()
                .flex()
                .items_start()
                .gap(rems(8. / 16.))
                .children(row),
        );
    }

    div()
        .debug_selector(move || selector.into())
        .flex()
        .flex_col()
        .gap(rems(10. / 16.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .text_size(theme::text_body())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::muted())
                        .child(heading),
                )
                .child(
                    div()
                        .debug_selector(move || meta_selector.into())
                        .text_size(theme::text_ui())
                        .text_color(theme::faint())
                        .child(meta),
                ),
        )
        .child(
            div()
                .w_full()
                .rounded(rems(12. / 16.))
                .border_1()
                .border_color(theme::border())
                .bg(theme::panel())
                .pt(rems(1.))
                .px(rems(12. / 16.))
                .pb(rems(14. / 16.))
                .flex()
                .flex_col()
                .gap(rems(12. / 16.))
                .children(rows),
        )
        .into_any_element()
}

fn is_plain_activation_key(event: &KeyDownEvent) -> bool {
    let modifiers = event.keystroke.modifiers;
    !event.is_held
        && !(modifiers.control || modifiers.platform || modifiers.alt || modifiers.function)
        && matches!(event.keystroke.key.as_str(), "enter" | "space")
}

// Paint opaque panel inside the focus ring so the drop shadow does not flood the footer text.
pub(super) fn theme_card_focus_style(style: gpui::StyleRefinement) -> gpui::StyleRefinement {
    style
        .rounded(rems(8. / 16.))
        .bg(theme::panel())
        .shadow(focus_ring(theme::accent()))
}

fn theme_gallery_card(card: ThemeGalleryCard) -> AnyElement {
    let preview_colors = theme::colors_for(card.variant);
    let palette = card.variant.terminal_palette();
    let tokens = preview_tokens(card.variant, palette);
    let key = card.key;
    let mode_tag = card.mode_tag;
    let (preview_selector, terminal_selector) = theme_card_selectors(card.variant);
    let on_click = card.on_select.clone();
    let on_key = card.on_select;

    div()
        .id(SharedString::from(format!(
            "appearance-{}-theme-{key}",
            mode_tag.to_ascii_lowercase()
        )))
        .debug_selector(move || format!("APPEARANCE_{mode_tag}_THEME_{key}"))
        .tab_index(0)
        .flex_1()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap(rems(7. / 16.))
        .cursor_pointer()
        .focus_visible(theme_card_focus_style)
        .on_click(move |event, _, cx| {
            if !event.is_keyboard() {
                on_click(cx);
            }
        })
        .on_key_down(move |event: &KeyDownEvent, _, cx| {
            if is_plain_activation_key(event) {
                cx.stop_propagation();
                on_key(cx);
            }
        })
        .child(super::side_panel::snapshot(
            preview_selector,
            div()
                .w_full()
                .h(rems(86. / 16.))
                .rounded(rems(8. / 16.))
                .overflow_hidden()
                .bg(gpui::rgb(preview_colors.bg))
                .when(card.selected, |preview| {
                    preview.border_2().border_color(theme::accent())
                })
                .when(!card.selected, |preview| {
                    preview.border_1().border_color(theme::border())
                })
                .flex()
                .child(
                    div()
                        .w(rems(28. / 16.))
                        .h_full()
                        .flex_shrink_0()
                        .bg(gpui::rgb(preview_colors.sidebar))
                        .py(rems(10. / 16.))
                        .px(rems(5. / 16.))
                        .flex()
                        .flex_col()
                        .gap(rems(5. / 16.))
                        .child(preview_bar(14., 3., preview_colors.fg_3))
                        .child(
                            div()
                                .w_full()
                                .h(rems(8. / 16.))
                                .rounded(rems(2. / 16.))
                                .bg(gpui::rgb(preview_colors.sidebar_selected)),
                        )
                        .child(preview_bar(16., 3., preview_colors.fg_3))
                        .child(preview_bar(11., 3., preview_colors.fg_3)),
                )
                .child(
                    div()
                        .flex_1()
                        .h_full()
                        .min_w(px(0.))
                        .p(rems(6. / 16.))
                        .flex()
                        .flex_col()
                        .gap(rems(5. / 16.))
                        .child(
                            div()
                                .w_full()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(preview_bar(24., 3., preview_colors.fg))
                                .child(
                                    div()
                                        .w(rems(13. / 16.))
                                        .h(rems(5. / 16.))
                                        .rounded(rems(3. / 16.))
                                        .bg(gpui::rgb(preview_colors.accent)),
                                ),
                        )
                        .child(super::side_panel::snapshot(
                            terminal_selector,
                            div()
                                .w_full()
                                .flex_1()
                                .min_h(px(0.))
                                .p(rems(6. / 16.))
                                .rounded(rems(4. / 16.))
                                .bg(palette_rgb(palette.background))
                                .flex()
                                .flex_col()
                                .justify_between()
                                .child(preview_code_row([
                                    (6., tokens.green),
                                    (26., palette_rgb(palette.foreground)),
                                ]))
                                .child(preview_code_row([(36., tokens.comment)]))
                                .child(preview_code_row([
                                    (12., tokens.blue),
                                    (10., tokens.yellow),
                                    (14., palette_rgb(palette.foreground)),
                                ]))
                                .child(preview_code_row([
                                    (10., tokens.red),
                                    (18., tokens.magenta),
                                ])),
                        )),
                ),
        ))
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .justify_between()
                .gap(rems(4. / 16.))
                .child(
                    div()
                        .debug_selector(move || format!("APPEARANCE_THEME_LABEL_{key}"))
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_size(theme::text_ui())
                        .font_weight(if card.selected {
                            FontWeight::MEDIUM
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(if card.selected {
                            theme::text()
                        } else {
                            theme::muted()
                        })
                        .child(card.label),
                )
                .when(card.selected, |row| {
                    row.child(
                        svg()
                            .debug_selector(move || format!("APPEARANCE_THEME_CHECK_{key}"))
                            .path("check.svg")
                            .size(rems(13. / 16.))
                            .flex_shrink_0()
                            .text_color(theme::accent()),
                    )
                }),
        )
        .into_any_element()
}

struct PreviewTokens {
    green: gpui::Rgba,
    comment: gpui::Rgba,
    blue: gpui::Rgba,
    yellow: gpui::Rgba,
    red: gpui::Rgba,
    magenta: gpui::Rgba,
}

const fn theme_card_selectors(variant: theme::ThemeVariant) -> (&'static str, &'static str) {
    match variant {
        theme::ThemeVariant::RunnerLight => (
            "APPEARANCE_THEME_PREVIEW_runner-light",
            "APPEARANCE_THEME_TERMINAL_runner-light",
        ),
        theme::ThemeVariant::CatppuccinLatte => (
            "APPEARANCE_THEME_PREVIEW_catppuccin-latte",
            "APPEARANCE_THEME_TERMINAL_catppuccin-latte",
        ),
        theme::ThemeVariant::RosePineDawn => (
            "APPEARANCE_THEME_PREVIEW_rose-pine-dawn",
            "APPEARANCE_THEME_TERMINAL_rose-pine-dawn",
        ),
        theme::ThemeVariant::TokyoNightDay => (
            "APPEARANCE_THEME_PREVIEW_tokyo-night-day",
            "APPEARANCE_THEME_TERMINAL_tokyo-night-day",
        ),
        theme::ThemeVariant::GruvboxLight => (
            "APPEARANCE_THEME_PREVIEW_gruvbox-light",
            "APPEARANCE_THEME_TERMINAL_gruvbox-light",
        ),
        theme::ThemeVariant::SolarizedLight => (
            "APPEARANCE_THEME_PREVIEW_solarized-light",
            "APPEARANCE_THEME_TERMINAL_solarized-light",
        ),
        theme::ThemeVariant::Carbon => (
            "APPEARANCE_THEME_PREVIEW_carbon",
            "APPEARANCE_THEME_TERMINAL_carbon",
        ),
        theme::ThemeVariant::CatppuccinMocha => (
            "APPEARANCE_THEME_PREVIEW_catppuccin-mocha",
            "APPEARANCE_THEME_TERMINAL_catppuccin-mocha",
        ),
        theme::ThemeVariant::RosePineMoon => (
            "APPEARANCE_THEME_PREVIEW_rose-pine-moon",
            "APPEARANCE_THEME_TERMINAL_rose-pine-moon",
        ),
        theme::ThemeVariant::TokyoNight => (
            "APPEARANCE_THEME_PREVIEW_tokyo-night",
            "APPEARANCE_THEME_TERMINAL_tokyo-night",
        ),
        theme::ThemeVariant::GruvboxDark => (
            "APPEARANCE_THEME_PREVIEW_gruvbox-dark",
            "APPEARANCE_THEME_TERMINAL_gruvbox-dark",
        ),
        theme::ThemeVariant::SolarizedDark => (
            "APPEARANCE_THEME_PREVIEW_solarized-dark",
            "APPEARANCE_THEME_TERMINAL_solarized-dark",
        ),
    }
}

fn preview_tokens(
    variant: theme::ThemeVariant,
    palette: runner_terminal::palette::TerminalPalette,
) -> PreviewTokens {
    // Gruvbox keeps muted earth tones on ANSI 1..5 and its vivid accents on ANSI 9..13.
    let [red, green, yellow, blue, magenta] = match variant {
        theme::ThemeVariant::GruvboxLight | theme::ThemeVariant::GruvboxDark => [
            palette.ansi[9],
            palette.ansi[10],
            palette.ansi[11],
            palette.ansi[12],
            palette.ansi[13],
        ],
        _ => [
            palette.ansi[1],
            palette.ansi[2],
            palette.ansi[3],
            palette.ansi[4],
            palette.ansi[5],
        ],
    };
    PreviewTokens {
        green: palette_rgb(green),
        comment: palette_rgb(palette.ansi[8]),
        blue: palette_rgb(blue),
        yellow: palette_rgb(yellow),
        red: palette_rgb(red),
        magenta: palette_rgb(magenta),
    }
}

fn palette_rgb(rgb: alacritty_terminal::vte::ansi::Rgb) -> gpui::Rgba {
    gpui::rgb(u32::from(rgb.r) << 16 | u32::from(rgb.g) << 8 | u32::from(rgb.b))
}

fn preview_bar(width_px: f32, height_px: f32, color: u32) -> gpui::Div {
    div()
        .w(rems(width_px / 16.))
        .h(rems(height_px / 16.))
        .rounded(rems((height_px / 2.) / 16.))
        .bg(gpui::rgb(color))
}

fn preview_code_row<const N: usize>(bars: [(f32, gpui::Rgba); N]) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(rems(3. / 16.))
        .children(bars.into_iter().map(|(width_px, color)| {
            div()
                .w(rems(width_px / 16.))
                .h(rems(3. / 16.))
                .rounded(rems(1.5 / 16.))
                .bg(color)
        }))
}

fn stepper_value(value: String, unit: Option<&'static str>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(rems(3. / 16.))
        .font_family(theme::UI_MONOSPACE_FONT)
        .child(
            div()
                .text_size(theme::text_ui())
                .font_weight(FontWeight::MEDIUM)
                .child(value),
        )
        .children(unit.map(|unit| {
            div()
                .text_size(theme::text_caption())
                .text_color(theme::faint())
                .child(unit)
        }))
        .into_any_element()
}

/// A section label, with a platform tag when the section exists on one
/// platform only, above its own card.
pub(super) fn settings_section(
    label: &'static str,
    selector: &'static str,
    macos_only: bool,
    rows: Vec<AnyElement>,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(rems(8. / 16.))
        .debug_selector(move || selector.into())
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(8. / 16.))
                .text_size(theme::text_body())
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::muted())
                .child(label)
                .when(macos_only, |heading| {
                    heading.child(
                        div()
                            .px(rems(7. / 16.))
                            .py(rems(2. / 16.))
                            .rounded(rems(5. / 16.))
                            .border_1()
                            .border_color(theme::border())
                            .text_size(theme::text_caption())
                            .font_weight(FontWeight::NORMAL)
                            .child("macOS"),
                    )
                }),
        )
        .child(SettingsCard::new(rows))
        .into_any_element()
}

/// A stepper with its range shown to its left, as `8–32`.
fn ranged_stepper(range: String, stepper: Stepper) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(rems(10. / 16.))
        .child(
            div()
                .text_size(theme::text_meta())
                .text_color(theme::faint())
                .child(range),
        )
        .child(stepper)
        .into_any_element()
}

/// The presets, plus a saved value that is not one of them so the select can
/// still show it.
pub(super) fn terminal_scrollback_options(saved: usize) -> Vec<SelectOption> {
    let mut values = TERMINAL_SCROLLBACK_PRESETS.to_vec();
    if !values.contains(&saved) {
        values.push(saved);
        values.sort_unstable();
    }
    values
        .into_iter()
        .map(|lines| {
            SelectOption::new(
                lines.to_string(),
                crate::i18n::messagef(
                    "terminal.scrollback_lines",
                    &[("lines", grouped_thousands(lines))],
                ),
            )
        })
        .collect()
}

fn grouped_thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

pub(super) fn terminal_cursor_value(value: TerminalCursorStyle) -> &'static str {
    match value {
        TerminalCursorStyle::Block => "block",
        TerminalCursorStyle::Underline => "underline",
        TerminalCursorStyle::Bar => "bar",
    }
}

pub(super) fn parse_terminal_cursor(value: &str) -> Option<TerminalCursorStyle> {
    match value {
        "block" => Some(TerminalCursorStyle::Block),
        "underline" => Some(TerminalCursorStyle::Underline),
        "bar" => Some(TerminalCursorStyle::Bar),
        _ => None,
    }
}
